//! Screen-space skin scattering: `skin_scatter.wgsl`.
//!
//! Two runs over the **HDR** image straight after the shading pass, while it
//! still holds only what that pass lit: across into this pass's own target,
//! then down, back into the HDR target. Every pixel whose material is not skin
//! is copied through, so later passes keep reading one view.
//!
//! Off until a material with `subsurface` above zero has been uploaded
//! (`SomniumRenderer::upload_scene_materials` turns it on): a frame with no
//! skin in it should not pay for two full-screen passes. `SOMNIUM_SKIN_SCATTER=0`
//! keeps it off for a before-and-after.

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct ScatterParams {
    direction: [f32; 2],
    pixels_per_metre: f32,
    reach: f32,
}

pub struct SkinScatterPass {
    pipeline: wgpu::RenderPipeline,
    layout: wgpu::BindGroupLayout,
    /// Across (reads the HDR target) and down (reads this pass's own).
    bind_groups: Option<[wgpu::BindGroup; 2]>,
    params: [wgpu::Buffer; 2],
    view: wgpu::TextureView,
    format: wgpu::TextureFormat,
    /// Metres the red channel spreads where a material's scatter is 1. Skin's
    /// own is two to three millimetres. At six a face three metres off lost
    /// its lines to the blur and read as waxed.
    pub reach: f32,
    /// A skin material is loaded.
    pub enabled: bool,
    allowed: bool,
}

impl SkinScatterPass {
    pub fn new(
        device: &wgpu::Device,
        shaders: &crate::shaders::Shaders,
        global_layout: &wgpu::BindGroupLayout,
        format: wgpu::TextureFormat,
        width: u32,
        height: u32,
    ) -> Self {
        let texture = |binding: u32, sample_type: wgpu::TextureSampleType| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Texture {
                sample_type,
                view_dimension: wgpu::TextureViewDimension::D2,
                multisampled: false,
            },
            count: None,
        };
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("Skin Scatter BGL"),
            entries: &[
                texture(0, wgpu::TextureSampleType::Float { filterable: true }),
                texture(1, wgpu::TextureSampleType::Depth),
                texture(2, wgpu::TextureSampleType::Uint),
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
        let params = [0, 1].map(|_| {
            device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("Skin Scatter Params"),
                size: std::mem::size_of::<ScatterParams>() as u64,
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            })
        });
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("Skin Scatter Shader"),
            source: wgpu::ShaderSource::Wgsl(shaders.source_or_panic("skin_scatter.wgsl").into()),
        });
        let pl = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("Skin Scatter PL"),
            bind_group_layouts: &[Some(global_layout), Some(&layout)],
            immediate_size: 0,
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("Skin Scatter Pipeline"),
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
            bind_groups: None,
            params,
            view: Self::alloc(device, format, width, height),
            format,
            reach: 0.003,
            enabled: false,
            allowed: std::env::var("SOMNIUM_SKIN_SCATTER").as_deref() != Ok("0"),
        }
    }

    pub fn active(&self) -> bool {
        self.enabled && self.allowed && self.reach > 0.0
    }

    fn alloc(device: &wgpu::Device, format: wgpu::TextureFormat, width: u32, height: u32) -> wgpu::TextureView {
        device
            .create_texture(&wgpu::TextureDescriptor {
                label: Some("Skin Scatter Target"),
                size: wgpu::Extent3d {
                    width: width.max(1),
                    height: height.max(1),
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
                view_formats: &[],
            })
            .create_view(&wgpu::TextureViewDescriptor::default())
    }

    pub fn resize(&mut self, device: &wgpu::Device, width: u32, height: u32) {
        self.view = Self::alloc(device, self.format, width, height);
        self.bind_groups = None;
    }

    /// Scatter the light on skin in `hdr`, in place. `projection_y` is the
    /// projection matrix's `[1][1]`. Returns true when it ran.
    #[allow(clippy::too_many_arguments)]
    pub fn record(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        global_bind_group: &wgpu::BindGroup,
        hdr_view: &wgpu::TextureView,
        depth_view: &wgpu::TextureView,
        vis_view: &wgpu::TextureView,
        projection_y: f32,
        height: u32,
    ) -> bool {
        if !self.active() {
            return false;
        }
        if self.bind_groups.is_none() {
            let group = |colour: &wgpu::TextureView, params: &wgpu::Buffer| {
                device.create_bind_group(&wgpu::BindGroupDescriptor {
                    label: Some("Skin Scatter BG"),
                    layout: &self.layout,
                    entries: &[
                        wgpu::BindGroupEntry {
                            binding: 0,
                            resource: wgpu::BindingResource::TextureView(colour),
                        },
                        wgpu::BindGroupEntry {
                            binding: 1,
                            resource: wgpu::BindingResource::TextureView(depth_view),
                        },
                        wgpu::BindGroupEntry {
                            binding: 2,
                            resource: wgpu::BindingResource::TextureView(vis_view),
                        },
                        wgpu::BindGroupEntry {
                            binding: 3,
                            resource: params.as_entire_binding(),
                        },
                    ],
                })
            };
            self.bind_groups = Some([group(hdr_view, &self.params[0]), group(&self.view, &self.params[1])]);
        }
        let Some(groups) = self.bind_groups.as_ref() else {
            return false;
        };
        for (buffer, direction) in self.params.iter().zip([[1.0, 0.0], [0.0, 1.0]]) {
            queue.write_buffer(
                buffer,
                0,
                bytemuck::bytes_of(&ScatterParams {
                    direction,
                    pixels_per_metre: projection_y.abs() * height as f32 * 0.5,
                    reach: self.reach,
                }),
            );
        }
        for (group, target) in groups.iter().zip([&self.view, hdr_view]) {
            let mut rpass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("Skin Scatter Pass"),
                multiview_mask: None,
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: target,
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
            rpass.set_bind_group(0, global_bind_group, &[]);
            rpass.set_bind_group(1, group, &[]);
            rpass.draw(0..3, 0..1);
        }
        true
    }
}

#[cfg(test)]
mod tests {
    use super::ScatterParams;

    #[test]
    fn the_params_struct_is_sixteen_bytes() {
        assert_eq!(std::mem::size_of::<ScatterParams>(), 16);
    }
}
