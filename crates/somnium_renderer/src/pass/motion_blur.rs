//! Motion blur (Phase 24Z, completing it).
//!
//! Runs on the **HDR** image, before tone mapping and before TAA reads it as
//! history — a blur applied after tone mapping smears clipped highlights as
//! flat white instead of letting a bright streak stay bright, which is the
//! difference between a headlight trail and a grey smudge.
//!
//! Ping-pongs through its own target and copies back, the same shape the DoF
//! pass uses, so every later pass keeps reading one HDR view.

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct MotionBlurParams {
    inv_resolution: [f32; 2],
    strength: f32,
    samples: f32,
}

pub struct MotionBlurPass {
    pipeline: wgpu::RenderPipeline,
    layout: wgpu::BindGroupLayout,
    bind_group: Option<wgpu::BindGroup>,
    params: wgpu::Buffer,
    texture: wgpu::Texture,
    view: wgpu::TextureView,
    format: wgpu::TextureFormat,
    /// Shutter fraction. 0.5 is a 180° shutter — the film default, and what
    /// "cinematic" motion blur means when a camera person says it.
    pub shutter: f32,
    /// Taps per side of the gather.
    pub samples: u32,
    pub enabled: bool,
}

impl MotionBlurPass {
    pub fn new(
        device: &wgpu::Device,
        shaders: &crate::shaders::Shaders,
        format: wgpu::TextureFormat,
        width: u32,
        height: u32,
    ) -> Self {
        let tex_entry = |binding: u32, depth: bool| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Texture {
                sample_type: if depth {
                    wgpu::TextureSampleType::Depth
                } else {
                    wgpu::TextureSampleType::Float { filterable: true }
                },
                view_dimension: wgpu::TextureViewDimension::D2,
                multisampled: false,
            },
            count: None,
        };
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("Motion Blur BGL"),
            entries: &[
                tex_entry(0, false),
                tex_entry(1, false),
                tex_entry(2, true),
                wgpu::BindGroupLayoutEntry {
                    binding: 3,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
            ],
        });

        let params = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Motion Blur Params"),
            size: std::mem::size_of::<MotionBlurParams>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let (texture, view) = Self::alloc(device, format, width, height);

        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("Motion Blur Shader"),
            source: wgpu::ShaderSource::Wgsl(shaders.source_or_panic("motion_blur.wgsl").into()),
        });
        let pl = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("Motion Blur PL"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("Motion Blur Pipeline"),
            layout: Some(&pl),
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
                    format,
                    blend: Some(wgpu::BlendState::REPLACE),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
                compilation_options: Default::default(),
            }),
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            cache: None,
        });

        Self {
            pipeline,
            layout,
            bind_group: None,
            params,
            texture,
            view,
            format,
            shutter: 0.5,
            samples: 8,
            // Off by default. Motion blur is a strong look, and it is the one
            // effect that makes a still screenshot of a moving camera look
            // broken rather than better.
            enabled: std::env::var("SOMNIUM_MOTION_BLUR").as_deref() == Ok("1"),
        }
    }

    pub fn active(&self) -> bool {
        self.enabled && self.shutter > 0.0
    }

    fn alloc(
        device: &wgpu::Device,
        format: wgpu::TextureFormat,
        width: u32,
        height: u32,
    ) -> (wgpu::Texture, wgpu::TextureView) {
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("Motion Blur Target"),
            size: wgpu::Extent3d {
                width: width.max(1),
                height: height.max(1),
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                | wgpu::TextureUsages::TEXTURE_BINDING
                | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        (texture, view)
    }

    pub fn resize(&mut self, device: &wgpu::Device, width: u32, height: u32) {
        let (texture, view) = Self::alloc(device, self.format, width, height);
        self.texture = texture;
        self.view = view;
        self.bind_group = None;
    }

    /// Blur `hdr` in place. Returns true when it ran.
    #[allow(clippy::too_many_arguments)]
    pub fn record(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        hdr_view: &wgpu::TextureView,
        hdr_texture: &wgpu::Texture,
        velocity_view: &wgpu::TextureView,
        depth_view: &wgpu::TextureView,
        width: u32,
        height: u32,
    ) -> bool {
        if !self.active() {
            return false;
        }
        if self.bind_group.is_none() {
            self.bind_group = Some(device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("Motion Blur BG"),
                layout: &self.layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: wgpu::BindingResource::TextureView(hdr_view),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: wgpu::BindingResource::TextureView(velocity_view),
                    },
                    wgpu::BindGroupEntry {
                        binding: 2,
                        resource: wgpu::BindingResource::TextureView(depth_view),
                    },
                    wgpu::BindGroupEntry {
                        binding: 3,
                        resource: self.params.as_entire_binding(),
                    },
                ],
            }));
        }
        let Some(bind) = self.bind_group.as_ref() else {
            return false;
        };

        queue.write_buffer(
            &self.params,
            0,
            bytemuck::bytes_of(&MotionBlurParams {
                inv_resolution: [1.0 / width as f32, 1.0 / height as f32],
                strength: self.shutter.clamp(0.0, 1.0),
                samples: self.samples.clamp(1, 32) as f32,
            }),
        );

        {
            let mut rpass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("Motion Blur Pass"),
                multiview_mask: None,
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &self.view,
                    resolve_target: None,
                    depth_slice: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
            });
            rpass.set_pipeline(&self.pipeline);
            rpass.set_bind_group(0, bind, &[]);
            rpass.draw(0..3, 0..1);
        }

        // Back into the HDR target, so everything downstream keeps reading one
        // view rather than a target that alternates.
        encoder.copy_texture_to_texture(
            self.texture.as_image_copy(),
            hdr_texture.as_image_copy(),
            hdr_texture.size(),
        );
        true
    }
}

#[cfg(test)]
mod tests {
    use super::MotionBlurParams;

    #[test]
    fn the_params_struct_is_a_multiple_of_sixteen() {
        assert_eq!(std::mem::size_of::<MotionBlurParams>() % 16, 0);
    }
}
