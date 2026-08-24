//! Phase 11.5I: Selection outline pass.
//!
//! Two-subpass stencil technique (inspired by bevy_mod_outline):
//!
//! 1. **Stencil write sub-pass** — renders the selected entity's geometry into
//!    a dedicated `Depth24PlusStencil8` texture, writing `stencil = 1` for
//!    every covered pixel.  No color is written (write_mask = NONE).
//!
//! 2. **Outline sub-pass** — renders the same geometry again with vertices
//!    extruded along the clip-space projected normal.  The `StencilFaceState`
//!    uses `CompareFunction::NotEqual` so only pixels *outside* the entity
//!    footprint receive the outline color.
//!
//! The pass runs after post-process (onto the swapchain) and uses its own
//! depth+stencil texture so it does not disturb the visibility-pass depth.

#![allow(clippy::too_many_arguments)]

/// Uniform block uploaded each frame (160 bytes).
#[repr(C)]
#[derive(Copy, Clone, bytemuck::Pod, bytemuck::Zeroable)]
struct OutlineUniforms {
    view_proj: [f32; 16],    // offset   0, 64 bytes
    model: [f32; 16],        // offset  64, 64 bytes
    outline_color: [f32; 4], // offset 128, 16 bytes
    outline_width: f32,      // offset 144,  4 bytes
    vertex_offset: u32,      // offset 148,  4 bytes
    index_offset: u32,       // offset 152,  4 bytes
    _pad: u32,               // offset 156,  4 bytes
}

fn make_ds_texture(
    device: &wgpu::Device,
    width: u32,
    height: u32,
) -> (wgpu::Texture, wgpu::TextureView) {
    let tex = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("Outline DS Texture"),
        size: wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Depth24PlusStencil8,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
        view_formats: &[],
    });
    let view = tex.create_view(&wgpu::TextureViewDescriptor::default());
    (tex, view)
}

/// Stencil-based selection outline pass.
pub struct OutlinePass {
    /// Renders entity geometry, writing stencil=1 (no color output).
    stencil_pipeline: wgpu::RenderPipeline,
    /// Renders extruded geometry where stencil != 1, writing outline color.
    outline_pipeline: wgpu::RenderPipeline,
    /// Dedicated depth+stencil texture (Depth24PlusStencil8).
    _ds_tex: wgpu::Texture,
    ds_view: wgpu::TextureView,
    /// Per-frame uniform buffer (160 bytes).
    uniform_buf: wgpu::Buffer,
    bind_group: wgpu::BindGroup,
    width: u32,
    height: u32,
}

impl OutlinePass {
    pub fn new(
        device: &wgpu::Device,
        shaders: &crate::shaders::Shaders,
        surface_format: wgpu::TextureFormat,
        vertex_buf: &wgpu::Buffer,
        index_buf: &wgpu::Buffer,
        width: u32,
        height: u32,
    ) -> Self {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("Outline Shader"),
            source: wgpu::ShaderSource::Wgsl(shaders.source_or_panic("outline.wgsl").into()),
        });

        let uniform_buf = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Outline Uniform Buffer"),
            size: std::mem::size_of::<OutlineUniforms>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("Outline BGL"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::VERTEX | wgpu::ShaderStages::FRAGMENT,
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
                        ty: wgpu::BufferBindingType::Storage { read_only: true },
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
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

        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("Outline BindGroup"),
            layout: &bgl,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: uniform_buf.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: vertex_buf.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: index_buf.as_entire_binding(),
                },
            ],
        });

        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("Outline Pipeline Layout"),
            bind_group_layouts: &[Some(&bgl)],
            immediate_size: 0,
        });

        // ── Sub-pass 1: write stencil=1, no color output ──────────────────────
        let stencil_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("Outline Stencil Pipeline"),
            layout: Some(&pipeline_layout),
            multiview_mask: None,
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_stencil"),
                buffers: &[],
                compilation_options: Default::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_stencil"),
                targets: &[Some(wgpu::ColorTargetState {
                    format: surface_format,
                    blend: None,
                    write_mask: wgpu::ColorWrites::empty(),
                })],
                compilation_options: Default::default(),
            }),
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                cull_mode: None,
                ..Default::default()
            },
            depth_stencil: Some(wgpu::DepthStencilState {
                format: wgpu::TextureFormat::Depth24PlusStencil8,
                depth_write_enabled: Some(false),
                depth_compare: Some(wgpu::CompareFunction::Always),
                stencil: wgpu::StencilState {
                    front: wgpu::StencilFaceState {
                        compare: wgpu::CompareFunction::Always,
                        fail_op: wgpu::StencilOperation::Keep,
                        depth_fail_op: wgpu::StencilOperation::Keep,
                        pass_op: wgpu::StencilOperation::Replace,
                    },
                    back: wgpu::StencilFaceState::IGNORE,
                    read_mask: 0xff,
                    write_mask: 0xff,
                },
                bias: wgpu::DepthBiasState::default(),
            }),
            multisample: wgpu::MultisampleState::default(),
            cache: None,
        });

        // ── Sub-pass 2: extruded outline, stencil NOTEQUAL=1 ─────────────────
        let outline_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("Outline Draw Pipeline"),
            layout: Some(&pipeline_layout),
            multiview_mask: None,
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_outline"),
                buffers: &[],
                compilation_options: Default::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_outline"),
                targets: &[Some(wgpu::ColorTargetState {
                    format: surface_format,
                    blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
                compilation_options: Default::default(),
            }),
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                cull_mode: Some(wgpu::Face::Back),
                ..Default::default()
            },
            depth_stencil: Some(wgpu::DepthStencilState {
                format: wgpu::TextureFormat::Depth24PlusStencil8,
                depth_write_enabled: Some(false),
                depth_compare: Some(wgpu::CompareFunction::Always),
                stencil: wgpu::StencilState {
                    front: wgpu::StencilFaceState {
                        compare: wgpu::CompareFunction::NotEqual,
                        fail_op: wgpu::StencilOperation::Keep,
                        depth_fail_op: wgpu::StencilOperation::Keep,
                        pass_op: wgpu::StencilOperation::Keep,
                    },
                    back: wgpu::StencilFaceState::IGNORE,
                    read_mask: 0xff,
                    write_mask: 0x00,
                },
                bias: wgpu::DepthBiasState::default(),
            }),
            multisample: wgpu::MultisampleState::default(),
            cache: None,
        });

        let (ds_tex, ds_view) = make_ds_texture(device, width.max(1), height.max(1));

        Self {
            stencil_pipeline,
            outline_pipeline,
            _ds_tex: ds_tex,
            ds_view,
            uniform_buf,
            bind_group,
            width,
            height,
        }
    }

    /// Recreate the depth+stencil texture after a window resize.
    pub fn resize(&mut self, device: &wgpu::Device, width: u32, height: u32) {
        if width != self.width || height != self.height {
            let (tex, view) = make_ds_texture(device, width.max(1), height.max(1));
            self._ds_tex = tex;
            self.ds_view = view;
            self.width = width;
            self.height = height;
        }
    }

    /// Record outline rendering for a selected entity.
    pub fn record(
        &self,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        surface_view: &wgpu::TextureView,
        view_proj: glam::Mat4,
        model: glam::Mat4,
        vertex_offset: u32,
        index_offset: u32,
        index_count: u32,
        color: [f32; 4],
        width_ndc: f32,
    ) {
        let uniforms = OutlineUniforms {
            view_proj: view_proj.to_cols_array(),
            model: model.to_cols_array(),
            outline_color: color,
            outline_width: width_ndc,
            vertex_offset,
            index_offset,
            _pad: 0,
        };
        queue.write_buffer(&self.uniform_buf, 0, bytemuck::bytes_of(&uniforms));

        // ── Sub-pass 1: clear stencil, write 1 for entity pixels ─────────────
        {
            let mut rpass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("Outline: Stencil"),
                multiview_mask: None,
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: surface_view,
                    resolve_target: None,
                    depth_slice: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Load,
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &self.ds_view,
                    depth_ops: None,
                    stencil_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Clear(0),
                        store: wgpu::StoreOp::Store,
                    }),
                }),
                timestamp_writes: None,
                occlusion_query_set: None,
            });
            rpass.set_pipeline(&self.stencil_pipeline);
            rpass.set_bind_group(0, &self.bind_group, &[]);
            rpass.set_stencil_reference(1);
            rpass.draw(0..index_count, 0..1);
        }

        // ── Sub-pass 2: extruded geometry, stencil NOTEQUAL 1 → outline ──────
        {
            let mut rpass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("Outline: Draw"),
                multiview_mask: None,
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: surface_view,
                    resolve_target: None,
                    depth_slice: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Load,
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &self.ds_view,
                    depth_ops: None,
                    stencil_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Load,
                        store: wgpu::StoreOp::Store,
                    }),
                }),
                timestamp_writes: None,
                occlusion_query_set: None,
            });
            rpass.set_pipeline(&self.outline_pipeline);
            rpass.set_bind_group(0, &self.bind_group, &[]);
            rpass.set_stencil_reference(1);
            rpass.draw(0..index_count, 0..1);
        }
    }
}
