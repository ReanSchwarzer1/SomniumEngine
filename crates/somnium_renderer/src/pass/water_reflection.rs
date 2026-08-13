//! Ray-traced water reflections (Phase VV — Halcyon).
//!
//! Half-resolution compute: one GGX/mirror ray per pixel from the water
//! G-buffer, hit shading through the shared `rt_hit.wgsl` resolve, temporal
//! accumulation, then the water shading pass bilateral-upsamples the result.
//! Fully skipped when the device did not grant `EXPERIMENTAL_RAY_QUERY` or
//! when `SOMNIUM_RT_REFLECT=0`.

const REFLECT_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba16Float;

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct ReflectParams {
    inv_view_proj: [[f32; 4]; 4],
    view: [[f32; 4]; 4],
    camera_pos: [f32; 3],
    frame: u32,
    inv_half_res: [f32; 2],
    history_valid: f32,
    rt_strength: f32,
    roughness_skip: f32,
    enabled: f32,
    _pad: [f32; 2],
}

#[cfg(test)]
mod tests {
    use super::ReflectParams;

    #[test]
    fn the_params_struct_is_the_176_byte_uniform_layout() {
        assert_eq!(std::mem::size_of::<ReflectParams>(), 176);
        assert_eq!(std::mem::size_of::<ReflectParams>() % 16, 0);
    }
}

pub struct WaterReflectionPass {
    pipeline: Option<wgpu::ComputePipeline>,
    layout: Option<wgpu::BindGroupLayout>,
    params: Option<wgpu::Buffer>,
    sampler: wgpu::Sampler,
    current: Option<(wgpu::Texture, wgpu::TextureView)>,
    history: Option<(wgpu::Texture, wgpu::TextureView)>,
    dummy: wgpu::TextureView,
    supported: bool,
    pub enabled: bool,
    history_valid: bool,
    frame: u32,
    /// Foam / unresolved water above this roughness skips the ray (VV-E).
    pub roughness_skip: f32,
}

impl WaterReflectionPass {
    pub fn new(
        device: &wgpu::Device,
        global_layout: &wgpu::BindGroupLayout,
        supported: bool,
        width: u32,
        height: u32,
    ) -> Self {
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("Water reflection sampler"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::MipmapFilterMode::Linear,
            ..Default::default()
        });
        let dummy = dummy_texture(device);

        let mut pass = Self {
            pipeline: None,
            layout: None,
            params: None,
            sampler,
            current: None,
            history: None,
            dummy,
            supported,
            enabled: supported && std::env::var("SOMNIUM_RT_REFLECT").as_deref() != Ok("0"),
            history_valid: false,
            frame: 0,
            roughness_skip: 0.72,
        };
        pass.allocate(device, width, height);
        if !supported {
            return pass;
        }

        let source = format!(
            "{}\n{}\n{}\n{}\n{}\n{}",
            include_str!("../shaders/water_reflection.wgsl"),
            include_str!("../shaders/rt_hit.wgsl"),
            include_str!("../shaders/global_pool.wgsl"),
            include_str!("../shaders/brdf.wgsl"),
            include_str!("../shaders/hextile.wgsl"),
            include_str!("../shaders/terrain_material.wgsl"),
        );
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("water_reflection.wgsl"),
            source: wgpu::ShaderSource::Wgsl(source.into()),
        });

        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("Water reflection BGL"),
            entries: &[
                accel_entry(0),
                tex_entry(1, true),
                tex_entry(2, true),
                tex_entry(3, true),
                wgpu::BindGroupLayoutEntry {
                    binding: 4,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::Cube,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 5,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 6,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Depth,
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 7,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Comparison),
                    count: None,
                },
                tex_entry(8, true),
                wgpu::BindGroupLayoutEntry {
                    binding: 9,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::StorageTexture {
                        access: wgpu::StorageTextureAccess::WriteOnly,
                        format: REFLECT_FORMAT,
                        view_dimension: wgpu::TextureViewDimension::D2,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 10,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 11,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });

        let pl = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("Water reflection PL"),
            bind_group_layouts: &[Some(global_layout), Some(&layout)],
            immediate_size: 0,
        });
        let pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("Water reflection"),
            layout: Some(&pl),
            module: &shader,
            entry_point: Some("main"),
            compilation_options: wgpu::PipelineCompilationOptions::default(),
            cache: None,
        });

        pass.pipeline = Some(pipeline);
        pass.layout = Some(layout);
        pass.params = Some(device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Water reflection params"),
            size: std::mem::size_of::<ReflectParams>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        }));
        pass
    }

    pub fn supported(&self) -> bool {
        self.supported
    }

    pub fn current_view(&self) -> &wgpu::TextureView {
        self.current.as_ref().map(|(_, v)| v).unwrap_or(&self.dummy)
    }

    pub fn dummy_view(&self) -> &wgpu::TextureView {
        &self.dummy
    }

    pub fn sampler(&self) -> &wgpu::Sampler {
        &self.sampler
    }

    pub fn resize(&mut self, device: &wgpu::Device, width: u32, height: u32) {
        self.allocate(device, width, height);
        self.history_valid = false;
    }

    fn allocate(&mut self, device: &wgpu::Device, width: u32, height: u32) {
        let hw = (width / 2).max(1);
        let hh = (height / 2).max(1);
        self.current = Some(half_target(device, hw, hh, "Water reflection current"));
        self.history = Some(half_target(device, hw, hh, "Water reflection history"));
    }

    #[allow(clippy::too_many_arguments)]
    pub fn record(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        global_bind: &wgpu::BindGroup,
        tlas: &wgpu::Tlas,
        water_surface: &wgpu::TextureView,
        water_roughness: &wgpu::TextureView,
        velocity: &wgpu::TextureView,
        env_view: &wgpu::TextureView,
        env_sampler: &wgpu::Sampler,
        shadow_atlas: &wgpu::TextureView,
        shadow_sampler: &wgpu::Sampler,
        view: glam::Mat4,
        view_proj: glam::Mat4,
        camera_pos: glam::Vec3,
        strength: f32,
        overflowed: bool,
        width: u32,
        height: u32,
    ) -> bool {
        let kill = std::env::var("SOMNIUM_RT_REFLECT").as_deref() == Ok("0");
        let active = self.enabled && self.supported && !overflowed && strength > 0.001 && !kill;
        if !active {
            self.history_valid = false;
            self.frame = self.frame.wrapping_add(1);
            return false;
        }
        if self.history_valid {
            std::mem::swap(&mut self.current, &mut self.history);
        }
        let (
            Some(pipeline),
            Some(layout),
            Some(params_buf),
            Some((_, current_view)),
            Some((_, history_view)),
        ) = (
            self.pipeline.as_ref(),
            self.layout.as_ref(),
            self.params.as_ref(),
            self.current.as_ref(),
            self.history.as_ref(),
        )
        else {
            return false;
        };

        let hw = (width / 2).max(1);
        let hh = (height / 2).max(1);
        queue.write_buffer(
            params_buf,
            0,
            bytemuck::bytes_of(&ReflectParams {
                inv_view_proj: view_proj.inverse().to_cols_array_2d(),
                view: view.to_cols_array_2d(),
                camera_pos: camera_pos.to_array(),
                frame: self.frame,
                inv_half_res: [1.0 / hw as f32, 1.0 / hh as f32],
                history_valid: f32::from(u8::from(self.history_valid && active)),
                rt_strength: strength.clamp(0.0, 1.0),
                roughness_skip: self.roughness_skip,
                enabled: f32::from(u8::from(active)),
                _pad: [0.0; 2],
            }),
        );

        let bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("Water reflection BG"),
            layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::AccelerationStructure(tlas),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(water_surface),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::TextureView(water_roughness),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: wgpu::BindingResource::TextureView(velocity),
                },
                wgpu::BindGroupEntry {
                    binding: 4,
                    resource: wgpu::BindingResource::TextureView(env_view),
                },
                wgpu::BindGroupEntry {
                    binding: 5,
                    resource: wgpu::BindingResource::Sampler(env_sampler),
                },
                wgpu::BindGroupEntry {
                    binding: 6,
                    resource: wgpu::BindingResource::TextureView(shadow_atlas),
                },
                wgpu::BindGroupEntry {
                    binding: 7,
                    resource: wgpu::BindingResource::Sampler(shadow_sampler),
                },
                wgpu::BindGroupEntry {
                    binding: 8,
                    resource: wgpu::BindingResource::TextureView(history_view),
                },
                wgpu::BindGroupEntry {
                    binding: 9,
                    resource: wgpu::BindingResource::TextureView(current_view),
                },
                wgpu::BindGroupEntry {
                    binding: 10,
                    resource: params_buf.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 11,
                    resource: wgpu::BindingResource::Sampler(&self.sampler),
                },
            ],
        });

        {
            let mut cpass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("Water reflection"),
                timestamp_writes: None,
            });
            cpass.set_pipeline(pipeline);
            cpass.set_bind_group(0, global_bind, &[]);
            cpass.set_bind_group(1, &bind, &[]);
            cpass.dispatch_workgroups(hw.div_ceil(8), hh.div_ceil(8), 1);
        }

        self.history_valid = true;
        self.frame = self.frame.wrapping_add(1);
        true
    }
}

fn accel_entry(binding: u32) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::COMPUTE,
        ty: wgpu::BindingType::AccelerationStructure {
            vertex_return: false,
        },
        count: None,
    }
}

fn tex_entry(binding: u32, filterable: bool) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::COMPUTE,
        ty: wgpu::BindingType::Texture {
            sample_type: wgpu::TextureSampleType::Float { filterable },
            view_dimension: wgpu::TextureViewDimension::D2,
            multisampled: false,
        },
        count: None,
    }
}

fn half_target(
    device: &wgpu::Device,
    width: u32,
    height: u32,
    label: &str,
) -> (wgpu::Texture, wgpu::TextureView) {
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some(label),
        size: wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: REFLECT_FORMAT,
        usage: wgpu::TextureUsages::STORAGE_BINDING
            | wgpu::TextureUsages::TEXTURE_BINDING
            | wgpu::TextureUsages::COPY_SRC
            | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });
    let view = texture.create_view(&Default::default());
    (texture, view)
}

fn dummy_texture(device: &wgpu::Device) -> wgpu::TextureView {
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("Water reflection dummy"),
        size: wgpu::Extent3d {
            width: 1,
            height: 1,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: REFLECT_FORMAT,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });
    texture.create_view(&Default::default())
}
