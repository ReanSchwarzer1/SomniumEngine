//! Screen-space velocity buffer (Phase 24AD).
//!
//! One full-screen pass over the depth buffer, producing UV-space motion for
//! every pixel. See `shaders/velocity.wgsl` for what it covers and what it does
//! not, and for the Wicked reference it follows.

/// Rg16Float: two signed UV offsets, and half precision is ample for a
/// quantity clamped to ±1 screen.
const VELOCITY_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rg16Float;

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct VelocityParams {
    inv_view_proj: [[f32; 4]; 4],
    prev_view_proj: [[f32; 4]; 4],
    inv_resolution: [f32; 2],
    valid: f32,
    _pad: f32,
}

pub struct VelocityPass {
    pipeline: wgpu::RenderPipeline,
    layout: wgpu::BindGroupLayout,
    bind_group: wgpu::BindGroup,
    params: wgpu::Buffer,
    texture: wgpu::Texture,
    view: wgpu::TextureView,
    prev_view_proj: glam::Mat4,
    /// False until a previous frame exists to reproject to.
    history_valid: bool,
}

impl VelocityPass {
    pub fn new(
        device: &wgpu::Device,
        shaders: &crate::shaders::Shaders,
        depth_view: &wgpu::TextureView,
        width: u32,
        height: u32,
    ) -> Self {
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("Velocity BGL"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Depth,
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
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
            label: Some("Velocity Params"),
            size: std::mem::size_of::<VelocityParams>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let (texture, view) = Self::alloc(device, width, height);
        let bind_group = Self::make_bind_group(device, &layout, depth_view, &params);

        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("Velocity Shader"),
            source: wgpu::ShaderSource::Wgsl(shaders.source_or_panic("velocity.wgsl").into()),
        });
        let pl = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("Velocity PL"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("Velocity Pipeline"),
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
                    format: VELOCITY_FORMAT,
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
            bind_group,
            params,
            texture,
            view,
            prev_view_proj: glam::Mat4::IDENTITY,
            history_valid: false,
        }
    }

    pub fn view(&self) -> &wgpu::TextureView {
        &self.view
    }

    pub fn texture(&self) -> &wgpu::Texture {
        &self.texture
    }

    fn alloc(device: &wgpu::Device, width: u32, height: u32) -> (wgpu::Texture, wgpu::TextureView) {
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("Velocity"),
            size: wgpu::Extent3d {
                width: width.max(1),
                height: height.max(1),
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: VELOCITY_FORMAT,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        (texture, view)
    }

    fn make_bind_group(
        device: &wgpu::Device,
        layout: &wgpu::BindGroupLayout,
        depth_view: &wgpu::TextureView,
        params: &wgpu::Buffer,
    ) -> wgpu::BindGroup {
        device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("Velocity BG"),
            layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(depth_view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: params.as_entire_binding(),
                },
            ],
        })
    }

    pub fn resize(
        &mut self,
        device: &wgpu::Device,
        depth_view: &wgpu::TextureView,
        width: u32,
        height: u32,
    ) {
        let (texture, view) = Self::alloc(device, width, height);
        self.texture = texture;
        self.view = view;
        self.bind_group = Self::make_bind_group(device, &self.layout, depth_view, &self.params);
        // The depth this reprojects from is a different image now.
        self.history_valid = false;
    }

    /// `view_proj_unjittered` must be un-jittered at both ends — see the
    /// shader's note and `TaaPass::record`, which measured what happens when it
    /// is not.
    pub fn record(
        &mut self,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        view_proj_unjittered: glam::Mat4,
        width: u32,
        height: u32,
    ) {
        queue.write_buffer(
            &self.params,
            0,
            bytemuck::bytes_of(&VelocityParams {
                inv_view_proj: view_proj_unjittered.inverse().to_cols_array_2d(),
                prev_view_proj: self.prev_view_proj.to_cols_array_2d(),
                inv_resolution: [1.0 / width as f32, 1.0 / height as f32],
                valid: f32::from(u8::from(self.history_valid)),
                _pad: 0.0,
            }),
        );

        let mut rpass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("Velocity Pass"),
            multiview_mask: None,
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: &self.view,
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
        });
        rpass.set_pipeline(&self.pipeline);
        rpass.set_bind_group(0, &self.bind_group, &[]);
        rpass.draw(0..3, 0..1);
        drop(rpass);

        self.prev_view_proj = view_proj_unjittered;
        self.history_valid = true;
    }
}

#[cfg(test)]
mod tests {
    use super::VelocityParams;

    #[test]
    fn the_params_struct_is_a_multiple_of_sixteen() {
        // Uniform structs must be, and a trailing scalar pair is exactly where
        // that goes wrong — see `wgpu_api_gotchas.md`.
        assert_eq!(std::mem::size_of::<VelocityParams>() % 16, 0);
        assert_eq!(std::mem::size_of::<VelocityParams>(), 144);
    }
}
