//! Visibility Buffer rendering pass.
//!
//! ## Reference Architecture
//!
//! Inspired by The Forge's `IVisibilityBuffer`.
//! Instead of a traditional G-Buffer (which suffers from overdraw bandwidth),
//! the visibility pass renders only the Triangle ID and Instance ID
//! into a high-precision texture. Subsequent compute passes use these
//! IDs to fetch the exact vertex data, evaluate materials, and output
//! the final lit pixel.

use wgpu;

/// Depth format for the scene buffer this pass fills.
///
/// Named (Phase DOOM-E) because the shading pass now attaches the same buffer
/// read-only for its depth split, and a pipeline whose depth format disagrees
/// with the attachment fails at creation rather than at the wrong pixel.
pub const DEPTH_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Depth32Float;

/// Levels of depth tie in each size class (renderer.rs `depth_tie`): larger things take `0..LEVELS`, smaller ones
/// `LEVELS..2 * LEVELS`.
pub const DEPTH_TIE_LEVELS: u32 = 8;
/// The largest step of tie in depth (depth_tie.wgsl `DEPTH_TIE_STEP_MAX`).
pub const DEPTH_TIE_STEP_MAX: f32 = 2.0e-6;
/// The most a tie draws anything nearer, near the eye: glass's tie (one above the largest opaque one) in steps at their
/// largest. The occlusion tests allow for it, less as ties fade (depth_tie.wgsl `DEPTH_TIE_REACH`, culling.rs).
pub const DEPTH_TIE_REACH: f32 = 3.2e-5;
/// Ties are whole to 12 m of view depth and gone past 24 m (depth_tie.wgsl).
pub const DEPTH_TIE_FADE: [f32; 2] = [12.0, 24.0];

/// How much of a tie is left at view depth `w` (depth_tie.wgsl `depth_tie_fade`).
pub fn depth_tie_fade(w: f32) -> f32 {
    ((DEPTH_TIE_FADE[1] - w) / (DEPTH_TIE_FADE[1] - DEPTH_TIE_FADE[0])).clamp(0.0, 1.0)
}
/// One step of tie, in pixels: how far the rasterizer's snapping of corners to its sub-pixel grid can put two copies of
/// one plane apart, twice over. tests/depth_tie_jitter.rs holds ties one step apart with 1/128 and loses edge-on with
/// 1/256.
const DEPTH_TIE_SNAP: f32 = 1.0 / 64.0;

/// The view's `depth_tie_slope` (visibility.wgsl): one step of tie for a plane 1 m from the eye and edge-on to it. A
/// plane's depth runs across the screen at the near plane times a pixel's angle times how far it is turned from facing
/// the eye over its distance from the eye, per pixel, so two copies of it round apart by that times the sub-pixel snap:
/// a wall seen edge-on needs far more than a floor seen from standing height, and a wall faced next to nothing. 0 (the
/// shader's floor) without a perspective.
pub fn depth_tie_slope(proj: &glam::Mat4, render_height: u32) -> f32 {
    let near = proj.w_axis.z / proj.z_axis.z;
    let pixel = 2.0 / (proj.y_axis.y * render_height.max(1) as f32);
    let slope = DEPTH_TIE_SNAP * near * pixel;
    if proj.z_axis.w != 0.0 && slope.is_finite() && slope > 0.0 { slope } else { 0.0 }
}

/// Represents the high-resolution texture used to store the
/// Instance ID and Triangle ID.
pub struct VisibilityBufferPass {
    pub texture: wgpu::Texture,
    pub view: wgpu::TextureView,
    pub depth_texture: wgpu::Texture,
    pub depth_view: wgpu::TextureView,
    pub pipeline: wgpu::RenderPipeline,
    /// Phase 17D: same pipeline with back-face culling off, for glTF
    /// `doubleSided` materials.
    pub pipeline_two_sided: wgpu::RenderPipeline,
    /// Sampler used by the alpha-cutout test, and each instance's depth tie (`set_depth_ties`).
    pub cutout_bind_group: wgpu::BindGroup,
    cutout_layout: wgpu::BindGroupLayout,
    cutout_sampler: wgpu::Sampler,
    depth_ties: wgpu::Buffer,
}

fn depth_ties_buffer(device: &wgpu::Device, bytes: u64) -> wgpu::Buffer {
    device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("Visibility Depth Ties"),
        size: bytes.max(4),
        usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    })
}

fn cutout_bind_group(
    device: &wgpu::Device,
    layout: &wgpu::BindGroupLayout,
    sampler: &wgpu::Sampler,
    depth_ties: &wgpu::Buffer,
) -> wgpu::BindGroup {
    device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("Visibility Cutout BG"),
        layout,
        entries: &[
            wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::Sampler(sampler) },
            wgpu::BindGroupEntry { binding: 1, resource: depth_ties.as_entire_binding() },
        ],
    })
}

impl VisibilityBufferPass {
    pub fn new(
        device: &wgpu::Device,
        shaders: &crate::shaders::Shaders,
        width: u32,
        height: u32,
        global_bind_group_layout: &wgpu::BindGroupLayout,
    ) -> Self {
        let size = wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        };

        // Rg32Uint: instance id in .r, primitive id in .g.
        //
        // This was R32Uint with both packed into 32 bits, which forces a
        // trade-off with no good answer. A 16/16 split capped meshes at 65 536
        // triangles and shattered the island tree's 714 000-triangle leaf mesh;
        // moving to 20 bits of primitive fixed that and capped instances at
        // 4 095, which a densely painted foliage scene blows through in turn —
        // and then *every* mesh fetches another instance's vertices, which is
        // far worse. Two full channels cost 4 bytes per pixel more and remove
        // both limits, so neither failure can come back.
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("VisibilityBuffer_Texture"),
            size,
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rg32Uint,
            // COPY_SRC so the frame capture can read the buffer back and tell
            // which pixels belong to which instance — the only way to say
            // "this changed *on terrain*" rather than "the image changed".
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                | wgpu::TextureUsages::TEXTURE_BINDING
                | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());

        let depth_texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("VisibilityBuffer_Depth"),
            size,
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Depth32Float,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });
        let depth_view = depth_texture.create_view(&wgpu::TextureViewDescriptor::default());

        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("Visibility Shader"),
            source: wgpu::ShaderSource::Wgsl(shaders.source_or_panic("visibility.wgsl").into()),
        });

        // Phase 17D: alpha cutout samples the albedo map here, which needs a
        // sampler the visibility pass never had.
        let cutout_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("Visibility Cutout BGL"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
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
            ],
        });
        let cutout_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("Visibility Cutout Sampler"),
            address_mode_u: wgpu::AddressMode::Repeat,
            address_mode_v: wgpu::AddressMode::Repeat,
            address_mode_w: wgpu::AddressMode::Repeat,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::MipmapFilterMode::Linear,
            ..Default::default()
        });
        let depth_ties = depth_ties_buffer(device, 4);
        let cutout_bind_group = cutout_bind_group(device, &cutout_layout, &cutout_sampler, &depth_ties);

        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("Visibility Pipeline Layout"),
            bind_group_layouts: &[Some(global_bind_group_layout), Some(&cutout_layout)],
            immediate_size: 0,
        });

        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("Visibility Pipeline"),
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
                    format: wgpu::TextureFormat::Rg32Uint,
                    blend: None,
                    write_mask: wgpu::ColorWrites::ALL,
                })],
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
                bias: wgpu::DepthBiasState::default(),
            }),
            multisample: wgpu::MultisampleState {
                count: 1,
                mask: !0,
                alpha_to_coverage_enabled: false,
            },
            cache: None,
        });

        // Phase 17D: the same pipeline with culling off, for glTF
        // `doubleSided` materials. A leaf card is a single flat quad — with
        // back-face culling it disappears entirely from one side.
        let mut two_sided_desc = wgpu::RenderPipelineDescriptor {
            label: Some("Visibility Pipeline (two-sided)"),
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
                    format: wgpu::TextureFormat::Rg32Uint,
                    blend: None,
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
            depth_stencil: Some(wgpu::DepthStencilState {
                format: wgpu::TextureFormat::Depth32Float,
                depth_write_enabled: Some(true),
                depth_compare: Some(wgpu::CompareFunction::Less),
                stencil: wgpu::StencilState::default(),
                bias: wgpu::DepthBiasState::default(),
            }),
            multisample: wgpu::MultisampleState {
                count: 1,
                mask: !0,
                alpha_to_coverage_enabled: false,
            },
            cache: None,
        };
        two_sided_desc.label = Some("Visibility Pipeline (two-sided)");
        let pipeline_two_sided = device.create_render_pipeline(&two_sided_desc);

        Self {
            texture,
            view,
            depth_texture,
            depth_view,
            pipeline,
            pipeline_two_sided,
            cutout_bind_group,
            cutout_layout,
            cutout_sampler,
            depth_ties,
        }
    }

    /// This frame's depth tie per opaque instance, in instance order (renderer.rs `depth_tie`; visibility.wgsl).
    pub fn set_depth_ties(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, ties: &[u32]) {
        let bytes = (ties.len().max(1) * 4) as u64;
        if self.depth_ties.size() < bytes {
            self.depth_ties = depth_ties_buffer(device, bytes.next_power_of_two());
            self.cutout_bind_group =
                cutout_bind_group(device, &self.cutout_layout, &self.cutout_sampler, &self.depth_ties);
        }
        if !ties.is_empty() {
            queue.write_buffer(&self.depth_ties, 0, bytemuck::cast_slice(ties));
        }
    }

    pub fn resize(
        &mut self,
        device: &wgpu::Device,
        shaders: &crate::shaders::Shaders,
        width: u32,
        height: u32,
        global_bind_group_layout: &wgpu::BindGroupLayout,
    ) {
        *self = Self::new(device, shaders, width, height, global_bind_group_layout);
    }
}
