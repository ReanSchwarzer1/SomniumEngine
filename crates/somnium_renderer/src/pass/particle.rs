//! Depth-tested textured particles in HDR. Each viewport owns its upload buffers.
#[repr(C)]
#[derive(Copy, Clone, Debug, bytemuck::Pod, bytemuck::Zeroable)]
pub struct GpuParticle {
    pub position: [f32; 3],
    /// Full sprite width in metres.
    pub size: f32,
    pub color: [f32; 4],
    pub tip_tint: [f32; 3],
    pub aspect: f32,
    pub uv_rect: [f32; 4],
    pub rotation: f32,
    pub texture_index: i32,
    /// Bit 0 additive; bit 1 world-up billboard.
    pub flags: u32,
    pub _pad: u32,
}
#[repr(C)]
#[derive(Copy, Clone, bytemuck::Pod, bytemuck::Zeroable)]
struct ParticleView {
    view_proj: [f32; 16],
    camera_right: [f32; 3],
    _pad0: f32,
    camera_up: [f32; 3],
    _pad1: f32,
}
const MAX_PARTICLES: usize = 10_000;
struct ViewBuffers {
    view: wgpu::Buffer,
    instances: wgpu::Buffer,
    group: wgpu::BindGroup,
    reactive: wgpu::Texture,
    reactive_view: wgpu::TextureView,
}
pub struct ParticlePass {
    pipeline: wgpu::RenderPipeline,
    layout: wgpu::BindGroupLayout,
    sampler: wgpu::Sampler,
    views: Vec<ViewBuffers>,
}
impl ParticlePass {
    pub fn new(
        device: &wgpu::Device,
        shaders: &crate::shaders::Shaders,
        global: &wgpu::BindGroupLayout,
    ) -> Self {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("Textured particles"),
            source: wgpu::ShaderSource::Wgsl(shaders.source_or_panic("particle.wgsl").into()),
        });
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("Particle view and instances"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::VERTEX,
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
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("Particle pipeline layout"),
            bind_group_layouts: &[Some(&layout), Some(global)],
            immediate_size: 0,
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("Textured HDR particles"),
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
                targets: &[
                    Some(wgpu::ColorTargetState {
                        format: crate::pass::postprocess::HDR_FORMAT,
                        blend: Some(wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING),
                        write_mask: wgpu::ColorWrites::ALL,
                    }),
                    Some(wgpu::ColorTargetState {
                        format: wgpu::TextureFormat::Rgba8Unorm,
                        blend: Some(wgpu::BlendState {
                            color: wgpu::BlendComponent {
                                src_factor: wgpu::BlendFactor::One,
                                dst_factor: wgpu::BlendFactor::One,
                                operation: wgpu::BlendOperation::Max,
                            },
                            alpha: wgpu::BlendComponent::REPLACE,
                        }),
                        write_mask: wgpu::ColorWrites::RED,
                    }),
                ],
                compilation_options: Default::default(),
            }),
            primitive: wgpu::PrimitiveState {
                cull_mode: None,
                ..Default::default()
            },
            depth_stencil: Some(wgpu::DepthStencilState {
                format: crate::pass::visibility::DEPTH_FORMAT,
                depth_write_enabled: Some(false),
                depth_compare: Some(wgpu::CompareFunction::LessEqual),
                stencil: Default::default(),
                bias: Default::default(),
            }),
            multisample: Default::default(),
            cache: None,
        });
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("Particle sprite sampler"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        Self {
            pipeline,
            layout,
            sampler,
            views: vec![],
        }
    }
    fn ensure_view(&mut self, device: &wgpu::Device, index: usize, size: [u32; 2]) {
        while self.views.len() <= index {
            let view = device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("Particle view"),
                size: std::mem::size_of::<ParticleView>() as u64,
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
            let instances = device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("Particle instances"),
                size: (MAX_PARTICLES * std::mem::size_of::<GpuParticle>()) as u64,
                usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
            let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("Particle view resources"),
                layout: &self.layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: view.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: instances.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 2,
                        resource: wgpu::BindingResource::Sampler(&self.sampler),
                    },
                ],
            });
            let reactive = reactive_target(device, size);
            let reactive_view = reactive.create_view(&Default::default());
            self.views.push(ViewBuffers {
                view,
                instances,
                group,
                reactive,
                reactive_view,
            });
        }
        let buffers = &mut self.views[index];
        if [buffers.reactive.width(), buffers.reactive.height()] != size {
            buffers.reactive = reactive_target(device, size);
            buffers.reactive_view = buffers.reactive.create_view(&Default::default());
        }
    }
    /// This frame's depth-tested sprite coverage; cleared even when emission stops.
    pub fn reactive_view(&self, slot: usize) -> Option<&wgpu::TextureView> {
        self.views.get(slot).map(|view| &view.reactive_view)
    }
    #[allow(clippy::too_many_arguments)]
    pub fn record(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        hdr: &wgpu::TextureView,
        depth: &wgpu::TextureView,
        global: &wgpu::BindGroup,
        view_proj: glam::Mat4,
        view: glam::Mat4,
        slot: usize,
        size: [u32; 2],
        particles: &[GpuParticle],
    ) {
        if slot >= 16 {
            return;
        }
        self.ensure_view(device, slot, size);
        let buffers = &self.views[slot];
        let right = view.row(0).truncate();
        let up = view.row(1).truncate();
        queue.write_buffer(
            &buffers.view,
            0,
            bytemuck::bytes_of(&ParticleView {
                view_proj: view_proj.to_cols_array(),
                camera_right: right.to_array(),
                _pad0: 0.0,
                camera_up: up.to_array(),
                _pad1: 0.0,
            }),
        );
        let mut sorted: Vec<_> = particles
            .iter()
            .copied()
            .filter(|p| {
                p.position.iter().all(|x| x.is_finite())
                    && p.size.is_finite()
                    && p.color.iter().all(|x| x.is_finite())
            })
            .take(MAX_PARTICLES)
            .collect();
        sorted.sort_by(|a, b| {
            view.transform_point3(glam::Vec3::from(a.position))
                .z
                .total_cmp(&view.transform_point3(glam::Vec3::from(b.position)).z)
        });
        if !sorted.is_empty() {
            queue.write_buffer(&buffers.instances, 0, bytemuck::cast_slice(&sorted));
        }
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("Textured HDR particles"),
            multiview_mask: None,
            color_attachments: &[
                Some(wgpu::RenderPassColorAttachment {
                    view: hdr,
                    resolve_target: None,
                    depth_slice: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Load,
                        store: wgpu::StoreOp::Store,
                    },
                }),
                Some(wgpu::RenderPassColorAttachment {
                    view: &buffers.reactive_view,
                    resolve_target: None,
                    depth_slice: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                        store: wgpu::StoreOp::Store,
                    },
                }),
            ],
            depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                view: depth,
                depth_ops: None,
                stencil_ops: None,
            }),
            timestamp_writes: None,
            occlusion_query_set: None,
        });
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, &buffers.group, &[]);
        pass.set_bind_group(1, global, &[]);
        pass.draw(0..6, 0..sorted.len() as u32);
    }
}

fn reactive_target(device: &wgpu::Device, size: [u32; 2]) -> wgpu::Texture {
    device.create_texture(&wgpu::TextureDescriptor {
        label: Some("Particle visible coverage"),
        size: wgpu::Extent3d {
            width: size[0].max(1),
            height: size[1].max(1),
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8Unorm,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT
            | wgpu::TextureUsages::TEXTURE_BINDING
            | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    })
}
