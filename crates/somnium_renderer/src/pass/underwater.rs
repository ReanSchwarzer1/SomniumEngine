//! Phase IV-G underwater RGB medium and partial-submersion composite.

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct UnderwaterParams {
    model: [[f32; 4]; 4],
    inverse_model: [[f32; 4]; 4],
    bounds: [f32; 4],
    absorption: [f32; 4],
    scattering: [f32; 4],
    surface: [f32; 4],
    wave_dirs: [f32; 4],
    wave: [f32; 4],
    frame: [f32; 4],
}

pub struct UnderwaterPass {
    pipeline: wgpu::RenderPipeline,
    layout: wgpu::BindGroupLayout,
    params: wgpu::Buffer,
    sampler: wgpu::Sampler,
    bind_group: Option<wgpu::BindGroup>,
    bound_water_id: Option<u32>,
}

impl UnderwaterPass {
    pub fn new(
        device: &wgpu::Device,
        shaders: &crate::shaders::Shaders, format: wgpu::TextureFormat) -> Self {
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("Underwater bind group layout"),
            entries: &[
                texture(0, true, false),
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
                texture(2, false, true),
                storage_buffer(3),
                storage_buffer(4),
                texture(5, false, false),
                texture(6, false, false),
                wgpu::BindGroupLayoutEntry {
                    binding: 7,
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
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("Underwater shader"),
            source: wgpu::ShaderSource::Wgsl(shaders.source_or_panic("underwater.wgsl").into()),
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("Underwater pipeline layout"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("Underwater medium pipeline"),
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
                    format,
                    blend: None,
                    write_mask: wgpu::ColorWrites::ALL,
                })],
                compilation_options: Default::default(),
            }),
            primitive: wgpu::PrimitiveState {
                cull_mode: None,
                ..Default::default()
            },
            depth_stencil: None,
            multisample: Default::default(),
            cache: None,
        });
        let params = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Underwater params"),
            size: std::mem::size_of::<UnderwaterParams>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("Underwater scene sampler"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        Self {
            pipeline,
            layout,
            params,
            sampler,
            bind_group: None,
            bound_water_id: None,
        }
    }

    pub fn invalidate(&mut self) {
        self.bind_group = None;
        self.bound_water_id = None;
    }

    #[allow(clippy::too_many_arguments)]
    pub fn record(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        target: &wgpu::TextureView,
        scene_copy: &wgpu::TextureView,
        depth: &wgpu::TextureView,
        view_buffer: &wgpu::Buffer,
        light_buffer: &wgpu::Buffer,
        water_id: u32,
        body: &crate::water_body::WaterBodyData,
        model: glam::Mat4,
        material: super::water::WaterMaterialData,
        time: f32,
        camera_signed_distance: f32,
    ) {
        if self.bind_group.is_none() || self.bound_water_id != Some(water_id) {
            self.bind_group = Some(device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("Underwater body bind group"),
                layout: &self.layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: wgpu::BindingResource::TextureView(scene_copy),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: wgpu::BindingResource::Sampler(&self.sampler),
                    },
                    wgpu::BindGroupEntry {
                        binding: 2,
                        resource: wgpu::BindingResource::TextureView(depth),
                    },
                    wgpu::BindGroupEntry {
                        binding: 3,
                        resource: view_buffer.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 4,
                        resource: light_buffer.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 5,
                        resource: wgpu::BindingResource::TextureView(&body.mask_view),
                    },
                    wgpu::BindGroupEntry {
                        binding: 6,
                        resource: wgpu::BindingResource::TextureView(&body.depth_view),
                    },
                    wgpu::BindGroupEntry {
                        binding: 7,
                        resource: self.params.as_entire_binding(),
                    },
                ],
            }));
            self.bound_water_id = Some(water_id);
        }
        queue.write_buffer(
            &self.params,
            0,
            bytemuck::bytes_of(&UnderwaterParams {
                model: model.to_cols_array_2d(),
                inverse_model: model.inverse().to_cols_array_2d(),
                bounds: material.bounds,
                absorption: material.absorption_roughness,
                scattering: material.scattering_anisotropy,
                surface: [
                    material.surface_params[2],
                    material.surface_params[0],
                    body.descriptor.max_depth,
                    material.volume_params[0],
                ],
                wave_dirs: [
                    material.wave_dir_a[0],
                    material.wave_dir_a[1],
                    material.wave_dir_b[0],
                    material.wave_dir_b[1],
                ],
                wave: material.wave_params,
                frame: [time, camera_signed_distance, material.volume_params[1], 0.0],
            }),
        );
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("Underwater medium"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: target,
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
            multiview_mask: None,
        });
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, self.bind_group.as_ref().unwrap(), &[]);
        pass.draw(0..3, 0..1);
    }
}

fn texture(binding: u32, filterable: bool, depth: bool) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::FRAGMENT,
        ty: wgpu::BindingType::Texture {
            sample_type: if depth {
                wgpu::TextureSampleType::Depth
            } else {
                wgpu::TextureSampleType::Float { filterable }
            },
            view_dimension: wgpu::TextureViewDimension::D2,
            multisampled: false,
        },
        count: None,
    }
}

fn storage_buffer(binding: u32) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::FRAGMENT,
        ty: wgpu::BindingType::Buffer {
            ty: wgpu::BufferBindingType::Storage { read_only: true },
            has_dynamic_offset: false,
            min_binding_size: None,
        },
        count: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn underwater_uniform_matches_wgsl_alignment() {
        assert_eq!(std::mem::size_of::<UnderwaterParams>(), 240);
        assert_eq!(std::mem::size_of::<UnderwaterParams>() % 16, 0);
    }
}
