//! Ground-truth ambient occlusion (Phase 24I).
//!
//! Produces a screen-space visibility term and a bent normal from depth alone.
//! Phase 17I applied only *baked* occlusion, so every surface without an AO map
//! — terrain, procedural meshes, all foliage — received sky light unattenuated.
//!
//! Runs after the visibility pass (which fills depth) and before shading, so
//! shading can fold the result into its indirect term.

/// AO and bent normal packed into one RGBA8 target: `rgb` = bent normal in
/// [0,1], `a` = visibility. One texture instead of two keeps the shading pass to
/// a single extra fetch.
const GTAO_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct GtaoParams {
    proj: [[f32; 4]; 4],
    inv_proj: [[f32; 4]; 4],
    inv_resolution: [f32; 2],
    radius: f32,
    power: f32,
    intensity: f32,
    frame: u32,
    near: f32,
    _pad: f32,
}

pub struct GtaoPass {
    trace_pipeline: wgpu::ComputePipeline,
    denoise_pipeline: wgpu::ComputePipeline,
    trace_layout: wgpu::BindGroupLayout,
    denoise_layout: wgpu::BindGroupLayout,
    params: wgpu::Buffer,

    raw_view: wgpu::TextureView,
    denoised_view: wgpu::TextureView,
    trace_bind: Option<wgpu::BindGroup>,
    denoise_bind: Option<wgpu::BindGroup>,

    frame: u32,
    pub enabled: bool,
    pub radius: f32,
    pub power: f32,
    pub intensity: f32,
}

impl GtaoPass {
    pub fn new(
        device: &wgpu::Device,
        shaders: &crate::shaders::Shaders,
        width: u32,
        height: u32,
    ) -> Self {
        // MORROWIND-C: composition is declared in `gtao.wgsl` and
        // resolved by `somnium_shader`; this site no longer knows the order.
        let source = shaders.source_or_panic("gtao.wgsl");
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("gtao.wgsl"),
            source: wgpu::ShaderSource::Wgsl(source.into()),
        });

        let depth_entry = |binding: u32| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::COMPUTE,
            ty: wgpu::BindingType::Texture {
                sample_type: wgpu::TextureSampleType::Depth,
                view_dimension: wgpu::TextureViewDimension::D2,
                multisampled: false,
            },
            count: None,
        };
        let storage_entry = |binding: u32| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::COMPUTE,
            ty: wgpu::BindingType::StorageTexture {
                access: wgpu::StorageTextureAccess::WriteOnly,
                format: GTAO_FORMAT,
                view_dimension: wgpu::TextureViewDimension::D2,
            },
            count: None,
        };

        let trace_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("GTAO trace BGL"),
            entries: &[
                depth_entry(0),
                storage_entry(1),
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
            ],
        });

        let denoise_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("GTAO denoise BGL"),
            entries: &[
                depth_entry(0),
                wgpu::BindGroupLayoutEntry {
                    binding: 3,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: false },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                storage_entry(4),
            ],
        });

        let make_pipeline = |layout: &wgpu::BindGroupLayout, entry: &str, label: &str| {
            let pl = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some(label),
                bind_group_layouts: &[Some(layout)],
                immediate_size: 0,
            });
            device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                label: Some(label),
                layout: Some(&pl),
                module: &shader,
                entry_point: Some(entry),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                cache: None,
            })
        };

        let (raw_view, denoised_view) = Self::make_targets(device, width, height);

        Self {
            trace_pipeline: make_pipeline(&trace_layout, "main", "GTAO trace"),
            denoise_pipeline: make_pipeline(&denoise_layout, "denoise", "GTAO denoise"),
            trace_layout,
            denoise_layout,
            params: device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("GTAO params"),
                size: std::mem::size_of::<GtaoParams>() as u64,
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            }),
            raw_view,
            denoised_view,
            trace_bind: None,
            denoise_bind: None,
            frame: 0,
            // Overwritten every frame from `PostProcessComponent::gtao_enabled`,
            // which is where the `SOMNIUM_GTAO` switch is seeded — a default
            // set here would never survive to the first frame.
            //
            // Off sets `intensity` to 0, which is `mix(1.0, ao, 0.0)` in the
            // shader: full visibility, bent normals still written, so only the
            // occlusion term changes.
            enabled: true,
            // A metre or so: large enough to darken where a trunk meets ground,
            // small enough that a wall does not shade the floor across a room.
            radius: 1.0,
            power: 2.0,
            intensity: 1.0,
        }
    }

    fn make_targets(
        device: &wgpu::Device,
        width: u32,
        height: u32,
    ) -> (wgpu::TextureView, wgpu::TextureView) {
        let make = |label: &str| {
            device
                .create_texture(&wgpu::TextureDescriptor {
                    label: Some(label),
                    size: wgpu::Extent3d {
                        width: width.max(1),
                        height: height.max(1),
                        depth_or_array_layers: 1,
                    },
                    mip_level_count: 1,
                    sample_count: 1,
                    dimension: wgpu::TextureDimension::D2,
                    format: GTAO_FORMAT,
                    usage: wgpu::TextureUsages::STORAGE_BINDING
                        | wgpu::TextureUsages::TEXTURE_BINDING,
                    view_formats: &[],
                })
                .create_view(&wgpu::TextureViewDescriptor::default())
        };
        (make("GTAO raw"), make("GTAO denoised"))
    }

    /// The texture shading should read: `rgb` bent normal, `a` visibility.
    pub fn output_view(&self) -> &wgpu::TextureView {
        &self.denoised_view
    }

    pub fn resize(&mut self, device: &wgpu::Device, width: u32, height: u32) {
        let (raw, denoised) = Self::make_targets(device, width, height);
        self.raw_view = raw;
        self.denoised_view = denoised;
        self.trace_bind = None;
        self.denoise_bind = None;
    }

    pub fn ensure_bind_groups(&mut self, device: &wgpu::Device, depth_view: &wgpu::TextureView) {
        if self.trace_bind.is_some() {
            return;
        }
        self.trace_bind = Some(device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("GTAO trace BG"),
            layout: &self.trace_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(depth_view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(&self.raw_view),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: self.params.as_entire_binding(),
                },
            ],
        }));
        self.denoise_bind = Some(device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("GTAO denoise BG"),
            layout: &self.denoise_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(depth_view),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: wgpu::BindingResource::TextureView(&self.raw_view),
                },
                wgpu::BindGroupEntry {
                    binding: 4,
                    resource: wgpu::BindingResource::TextureView(&self.denoised_view),
                },
            ],
        }));
    }

    pub fn record(
        &mut self,
        encoder: &mut wgpu::CommandEncoder,
        queue: &wgpu::Queue,
        proj: glam::Mat4,
        width: u32,
        height: u32,
        near: f32,
    ) {
        let (Some(trace), Some(denoise)) = (self.trace_bind.as_ref(), self.denoise_bind.as_ref())
        else {
            return;
        };

        queue.write_buffer(
            &self.params,
            0,
            bytemuck::bytes_of(&GtaoParams {
                proj: proj.to_cols_array_2d(),
                inv_proj: proj.inverse().to_cols_array_2d(),
                inv_resolution: [1.0 / width as f32, 1.0 / height as f32],
                radius: self.radius,
                power: self.power,
                intensity: if self.enabled { self.intensity } else { 0.0 },
                frame: self.frame,
                near,
                _pad: 0.0,
            }),
        );
        self.frame = self.frame.wrapping_add(1);

        let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
            label: Some("GTAO"),
            timestamp_writes: None,
        });
        let groups_x = width.div_ceil(8);
        let groups_y = height.div_ceil(8);

        pass.set_pipeline(&self.trace_pipeline);
        pass.set_bind_group(0, trace, &[]);
        pass.dispatch_workgroups(groups_x, groups_y, 1);

        pass.set_pipeline(&self.denoise_pipeline);
        pass.set_bind_group(0, denoise, &[]);
        pass.dispatch_workgroups(groups_x, groups_y, 1);
    }
}
