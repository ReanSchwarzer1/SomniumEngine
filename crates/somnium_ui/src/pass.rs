// Phase 12B-1 — UiPass: wgpu render pass for native UI draw lists.
//
// Uploads vertex/index data from DrawingContext each frame, maintains the
// font atlas as a GPU texture, and records one indexed draw per DrawCommand
// with a per-command scissor rect.
//
// Bind groups:
//   BG0 b0 (VERTEX):   mat4 ortho projection (screen-space, y-down).
//   BG1 b0+b1 (FRAG):  texture2d + sampler.  Two pre-built variants:
//     - bg1_white  — white 1×1 pixel  (DrawCommand::texture_id = None)
//     - bg1_atlas  — font atlas        (DrawCommand::texture_id = Some(0))
//   BG1 is switched only when the texture changes between commands.

use crate::{
    draw::{DrawCommand, DrawingContext, Vertex},
    font::{ATLAS_HEIGHT, ATLAS_WIDTH, FONT_ATLAS_TEXTURE_ID},
    icons::{ICON_ATLAS_HEIGHT, ICON_ATLAS_TEXTURE_ID, ICON_ATLAS_WIDTH},
};
use glam::Mat4;

const INIT_VTX_CAP: u64 = 65536 * 20; // 65 K vertices × 20 B
const INIT_IDX_CAP: u64 = 131072 * 4; // 128 K indices  × 4 B

pub struct UiPass {
    pipeline: wgpu::RenderPipeline,
    bg1_layout: wgpu::BindGroupLayout,
    // BG0 — ortho uniform
    ortho_buf: wgpu::Buffer,
    bg0: wgpu::BindGroup,
    // Shared sampler
    sampler: wgpu::Sampler,
    // BG1 variants
    _white_tex: wgpu::Texture, // kept alive so white_view stays valid
    _white_view: wgpu::TextureView,
    bg1_white: wgpu::BindGroup,
    atlas_tex: wgpu::Texture,
    atlas_view: wgpu::TextureView,
    bg1_atlas: wgpu::BindGroup,
    icon_tex: wgpu::Texture,
    icon_view: wgpu::TextureView,
    bg1_icon: wgpu::BindGroup,
    // Geometry buffers (recreated on overflow)
    vtx_buf: wgpu::Buffer,
    idx_buf: wgpu::Buffer,
    vtx_capacity: u64,
    idx_capacity: u64,
    // Draw list cached from the last prepare() call
    commands: Vec<DrawCommand>,
    surface_w: u32,
    surface_h: u32,
}

impl UiPass {
    pub fn new(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        surface_format: wgpu::TextureFormat,
    ) -> Self {
        // ── Bind group layouts ────────────────────────────────────────────────
        let bg0_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("UiPass BGL0"),
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

        let bg1_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("UiPass BGL1"),
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
            ],
        });

        // ── Ortho uniform buffer ──────────────────────────────────────────────
        let ortho_buf = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("UiPass Ortho"),
            size: 64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let bg0 = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("UiPass BG0"),
            layout: &bg0_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: ortho_buf.as_entire_binding(),
            }],
        });

        // ── Shared linear sampler ─────────────────────────────────────────────
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("UiPass Sampler"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });

        // ── White 1×1 texture (solid-colour rects) ────────────────────────────
        let white_tex = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("UiPass White"),
            size: wgpu::Extent3d {
                width: 1,
                height: 1,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &white_tex,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            &[255u8, 255, 255, 255],
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(4),
                rows_per_image: Some(1),
            },
            wgpu::Extent3d {
                width: 1,
                height: 1,
                depth_or_array_layers: 1,
            },
        );
        let white_view = white_tex.create_view(&wgpu::TextureViewDescriptor::default());

        // ── Font atlas texture (512×512, Rgba8Unorm) ──────────────────────────
        let atlas_tex = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("UiPass Font Atlas"),
            size: wgpu::Extent3d {
                width: ATLAS_WIDTH,
                height: ATLAS_HEIGHT,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        let atlas_view = atlas_tex.create_view(&wgpu::TextureViewDescriptor::default());

        let icon_tex = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("UiPass Icon Atlas"),
            size: wgpu::Extent3d {
                width: ICON_ATLAS_WIDTH,
                height: ICON_ATLAS_HEIGHT,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        let icon_view = icon_tex.create_view(&wgpu::TextureViewDescriptor::default());

        // ── BG1 bind groups ───────────────────────────────────────────────────
        let bg1_white = Self::make_bg1(device, &bg1_layout, &white_view, &sampler);
        let bg1_atlas = Self::make_bg1(device, &bg1_layout, &atlas_view, &sampler);
        let bg1_icon = Self::make_bg1(device, &bg1_layout, &icon_view, &sampler);

        // ── Geometry buffers ──────────────────────────────────────────────────
        let vtx_buf = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("UI Vertices"),
            size: INIT_VTX_CAP,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let idx_buf = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("UI Indices"),
            size: INIT_IDX_CAP,
            usage: wgpu::BufferUsages::INDEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        // ── Shader + pipeline ─────────────────────────────────────────────────
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("UiPass Shader"),
            source: wgpu::ShaderSource::Wgsl(include_str!("ui_pass.wgsl").into()),
        });

        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("UiPass Pipeline Layout"),
            bind_group_layouts: &[Some(&bg0_layout), Some(&bg1_layout)],
            immediate_size: 0,
        });

        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("UiPass Pipeline"),
            layout: Some(&pipeline_layout),
            multiview_mask: None,
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                compilation_options: Default::default(),
                buffers: &[wgpu::VertexBufferLayout {
                    array_stride: 20,
                    step_mode: wgpu::VertexStepMode::Vertex,
                    attributes: &[
                        wgpu::VertexAttribute {
                            shader_location: 0,
                            offset: 0,
                            format: wgpu::VertexFormat::Float32x2,
                        },
                        wgpu::VertexAttribute {
                            shader_location: 1,
                            offset: 8,
                            format: wgpu::VertexFormat::Float32x2,
                        },
                        wgpu::VertexAttribute {
                            shader_location: 2,
                            offset: 16,
                            format: wgpu::VertexFormat::Unorm8x4,
                        },
                    ],
                }],
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_main"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: surface_format,
                    blend: Some(wgpu::BlendState {
                        color: wgpu::BlendComponent {
                            src_factor: wgpu::BlendFactor::SrcAlpha,
                            dst_factor: wgpu::BlendFactor::OneMinusSrcAlpha,
                            operation: wgpu::BlendOperation::Add,
                        },
                        alpha: wgpu::BlendComponent {
                            src_factor: wgpu::BlendFactor::One,
                            dst_factor: wgpu::BlendFactor::OneMinusSrcAlpha,
                            operation: wgpu::BlendOperation::Add,
                        },
                    }),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                front_face: wgpu::FrontFace::Ccw,
                cull_mode: None,
                ..Default::default()
            },
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            cache: None,
        });

        Self {
            pipeline,
            bg1_layout,
            ortho_buf,
            bg0,
            sampler,
            _white_tex: white_tex,
            _white_view: white_view,
            bg1_white,
            atlas_tex,
            atlas_view,
            bg1_atlas,
            icon_tex,
            icon_view,
            bg1_icon,
            vtx_buf,
            idx_buf,
            vtx_capacity: INIT_VTX_CAP,
            idx_capacity: INIT_IDX_CAP,
            commands: Vec::new(),
            surface_w: 0,
            surface_h: 0,
        }
    }

    fn make_bg1(
        device: &wgpu::Device,
        layout: &wgpu::BindGroupLayout,
        view: &wgpu::TextureView,
        sampler: &wgpu::Sampler,
    ) -> wgpu::BindGroup {
        device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("UiPass BG1"),
            layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(sampler),
                },
            ],
        })
    }

    /// Upload draw data to the GPU. Call once per frame before `render()`.
    ///
    /// - Uploads ortho projection uniform.
    /// - Re-uploads the font atlas only when `draw_ctx.font_atlas.dirty` is set.
    /// - Grows vertex/index buffers if needed (doubling strategy).
    /// - Caches a copy of `draw_ctx.commands` for the subsequent `render()` call.
    pub fn prepare(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        draw_ctx: &mut DrawingContext,
        surface_w: u32,
        surface_h: u32,
    ) {
        self.surface_w = surface_w;
        self.surface_h = surface_h;

        // Ortho: (0,0) = top-left, (W,H) = bottom-right, y-down.
        let proj = Mat4::orthographic_rh(0.0, surface_w as f32, surface_h as f32, 0.0, 0.0, 1.0);
        queue.write_buffer(
            &self.ortho_buf,
            0,
            bytemuck::bytes_of(&proj.to_cols_array()),
        );

        // Font atlas — dirty flag cleared here so we don't re-upload next frame.
        if draw_ctx.font_atlas.dirty {
            queue.write_texture(
                wgpu::TexelCopyTextureInfo {
                    texture: &self.atlas_tex,
                    mip_level: 0,
                    origin: wgpu::Origin3d::ZERO,
                    aspect: wgpu::TextureAspect::All,
                },
                &draw_ctx.font_atlas.pixels,
                wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(ATLAS_WIDTH * 4),
                    rows_per_image: Some(ATLAS_HEIGHT),
                },
                wgpu::Extent3d {
                    width: ATLAS_WIDTH,
                    height: ATLAS_HEIGHT,
                    depth_or_array_layers: 1,
                },
            );
            draw_ctx.font_atlas.dirty = false;

            // Recreate bg1_atlas because the atlas_view is still pointing to the
            // same GPU texture — just rebind so the sampler is guaranteed fresh.
            self.bg1_atlas =
                Self::make_bg1(device, &self.bg1_layout, &self.atlas_view, &self.sampler);
        }

        if draw_ctx.icon_atlas.dirty {
            queue.write_texture(
                wgpu::TexelCopyTextureInfo {
                    texture: &self.icon_tex,
                    mip_level: 0,
                    origin: wgpu::Origin3d::ZERO,
                    aspect: wgpu::TextureAspect::All,
                },
                &draw_ctx.icon_atlas.pixels,
                wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(ICON_ATLAS_WIDTH * 4),
                    rows_per_image: Some(ICON_ATLAS_HEIGHT),
                },
                wgpu::Extent3d {
                    width: ICON_ATLAS_WIDTH,
                    height: ICON_ATLAS_HEIGHT,
                    depth_or_array_layers: 1,
                },
            );
            draw_ctx.icon_atlas.dirty = false;
            self.bg1_icon =
                Self::make_bg1(device, &self.bg1_layout, &self.icon_view, &self.sampler);
        }

        // Vertices
        if !draw_ctx.vertices.is_empty() {
            let needed = (draw_ctx.vertices.len() * std::mem::size_of::<Vertex>()) as u64;
            if needed > self.vtx_capacity {
                self.vtx_capacity = needed * 2;
                self.vtx_buf = device.create_buffer(&wgpu::BufferDescriptor {
                    label: Some("UI Vertices"),
                    size: self.vtx_capacity,
                    usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
                    mapped_at_creation: false,
                });
            }
            queue.write_buffer(&self.vtx_buf, 0, bytemuck::cast_slice(&draw_ctx.vertices));
        }

        // Indices
        if !draw_ctx.indices.is_empty() {
            let needed = (draw_ctx.indices.len() * 4) as u64;
            if needed > self.idx_capacity {
                self.idx_capacity = needed * 2;
                self.idx_buf = device.create_buffer(&wgpu::BufferDescriptor {
                    label: Some("UI Indices"),
                    size: self.idx_capacity,
                    usage: wgpu::BufferUsages::INDEX | wgpu::BufferUsages::COPY_DST,
                    mapped_at_creation: false,
                });
            }
            queue.write_buffer(&self.idx_buf, 0, bytemuck::cast_slice(&draw_ctx.indices));
        }

        self.commands = draw_ctx.commands.clone();
    }

    /// Record the UI render pass. Composites onto the existing surface contents.
    pub fn render(&self, encoder: &mut wgpu::CommandEncoder, surface_view: &wgpu::TextureView) {
        if self.commands.is_empty() {
            return;
        }

        let mut rpass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("UiPass"),
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
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
        });

        rpass.set_pipeline(&self.pipeline);
        rpass.set_vertex_buffer(0, self.vtx_buf.slice(..));
        rpass.set_index_buffer(self.idx_buf.slice(..), wgpu::IndexFormat::Uint32);
        rpass.set_bind_group(0, &self.bg0, &[]);

        // Start with white; switch lazily per DrawCommand.
        let mut active_tex: Option<u32> = None;
        rpass.set_bind_group(1, &self.bg1_white, &[]);

        let sw = self.surface_w;
        let sh = self.surface_h;

        for cmd in &self.commands {
            if cmd.index_count == 0 {
                continue;
            }

            // Scissor rect clamped to [0, surface_w) × [0, surface_h).
            let x0 = (cmd.clip_rect.x.max(0.0) as u32).min(sw);
            let y0 = (cmd.clip_rect.y.max(0.0) as u32).min(sh);
            let x1 = ((cmd.clip_rect.x + cmd.clip_rect.w).max(0.0) as u32).min(sw);
            let y1 = ((cmd.clip_rect.y + cmd.clip_rect.h).max(0.0) as u32).min(sh);
            let cw = x1.saturating_sub(x0);
            let ch = y1.saturating_sub(y0);
            if cw == 0 || ch == 0 {
                continue;
            }
            rpass.set_scissor_rect(x0, y0, cw, ch);

            // Switch BG1 only when the texture changes.
            if cmd.texture_id != active_tex {
                active_tex = cmd.texture_id;
                match active_tex {
                    None => rpass.set_bind_group(1, &self.bg1_white, &[]),
                    Some(id) if id == FONT_ATLAS_TEXTURE_ID => {
                        rpass.set_bind_group(1, &self.bg1_atlas, &[])
                    }
                    Some(id) if id == ICON_ATLAS_TEXTURE_ID => {
                        rpass.set_bind_group(1, &self.bg1_icon, &[])
                    }
                    Some(_) => rpass.set_bind_group(1, &self.bg1_atlas, &[]),
                }
            }

            rpass.draw_indexed(
                cmd.index_offset..(cmd.index_offset + cmd.index_count),
                0,
                0..1,
            );
        }
    }
}
