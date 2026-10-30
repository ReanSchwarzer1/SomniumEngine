//! Real-time path tracer and its denoiser (17 Oct 2026; `path_tracer.wgsl`, `path_denoise.wgsl`).
//!
//! Replaces the 24O accumulator in `lighting_extra` (a Lambert-only, sun-only average that paused the simulation to
//! gather): one path a pixel a frame with GGX specular, the sun, the moon and the level's lamps sampled directly,
//! then a reprojected temporal accumulation and an edge-stopping a-trous filter, so it runs while the game runs.
//!
//! `enabled` (mode 0, 29 Oct 2026) traces the bounced light from the raster's own surface: the shading pass, with the
//! path mode bit set, writes its finished material into `surface_views` and draws its direct light without the
//! ambient; `record` then runs after it, and `composite` adds the traced light over its picture. So every material
//! detail the raster draws (normal, roughness and metal maps, paint, baked light, decals, terrain layers) is the
//! traced picture's too.
//!
//! Off unless `enabled`; with no ray query on the device it never records, and `output_view` is a cleared target.
//!
//! `reflections` (17 Oct 2026) runs it in its second mode instead: ray-traced reflections on glossy surfaces only,
//! composited by shading through its traced-specular bit, with the raster's own lighting everywhere else.

const FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba16Float;
/// Frames a pixel's history may hold, and how hard the filter weighs light against light.
const MAX_HISTORY: f32 = 48.0;
const SIGMA_LIGHT: f32 = 4.0;
/// `SOMNIUM_PATH_DEBUG`: 1 magenta, 2 albedo, 3 unfiltered light, 4 glow (path_denoise.wgsl `remodulate`).
static DEBUG: std::sync::LazyLock<u32> =
    std::sync::LazyLock::new(|| std::env::var("SOMNIUM_PATH_DEBUG").ok().and_then(|v| v.parse().ok()).unwrap_or(0));
/// The a-trous steps: three passes, 1, 2 and 4 pixels apart.
const STEPS: [u32; 3] = [1, 2, 4];
/// Reflections: the roughest surface that traces one by default (path_denoise.wgsl fades it out over the last 30%),
/// and the history a reflection keeps (short: it moves across the surface as the eye does, which the surface's motion
/// cannot follow).
pub const REFLECT_MAX_ROUGH: f32 = 0.5;
const REFLECT_HISTORY: f32 = 8.0;
/// `SOMNIUM_RT_REFLECT_ROUGH=1` (review only) overrides the authored limit: every surface traces its reflection.
static REFLECT_ROUGH: std::sync::LazyLock<Option<f32>> =
    std::sync::LazyLock::new(|| std::env::var("SOMNIUM_RT_REFLECT_ROUGH").ok().and_then(|v| v.parse().ok()));

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct PtParams {
    inv_view_proj: [[f32; 4]; 4],
    camera_pos: [f32; 3],
    frame: u32,
    bounces: u32,
    lamp_count: u32,
    firefly: f32,
    env_intensity: f32,
    mode: u32,
    max_rough: f32,
    _pad: [u32; 2],
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct DnParams {
    inv_res: [f32; 2],
    step: u32,
    reset: u32,
    max_history: f32,
    sigma_light: f32,
    _pad: [f32; 2],
}

struct Target {
    texture: wgpu::Texture,
    view: wgpu::TextureView,
}

fn target(device: &wgpu::Device, width: u32, height: u32, label: &str) -> Target {
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some(label),
        size: wgpu::Extent3d { width: width.max(1), height: height.max(1), depth_or_array_layers: 1 },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: FORMAT,
        usage: wgpu::TextureUsages::STORAGE_BINDING
            | wgpu::TextureUsages::TEXTURE_BINDING
            | wgpu::TextureUsages::COPY_SRC
            | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });
    let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
    Target { texture, view }
}

struct Targets {
    light: Target,
    gbuf: Target,
    gbuf_prev: Target,
    albedo: Target,
    accum: [Target; 2],
    ping: Target,
    pong: Target,
    output: Target,
    /// The raster's surface (shading.wgsl `path_surface_*`): albedo and metallic, normal and roughness, occlusion
    /// and the fog's transmittance.
    surface: [Target; 3],
}

impl Targets {
    fn new(device: &wgpu::Device, width: u32, height: u32) -> Self {
        Self {
            light: target(device, width, height, "Path tracer light"),
            gbuf: target(device, width, height, "Path tracer normal and distance"),
            gbuf_prev: target(device, width, height, "Path tracer last normal and distance"),
            albedo: target(device, width, height, "Path tracer albedo"),
            accum: [target(device, width, height, "Path tracer history A"), target(device, width, height, "Path tracer history B")],
            ping: target(device, width, height, "Path tracer filter A"),
            pong: target(device, width, height, "Path tracer filter B"),
            output: target(device, width, height, "Path tracer output"),
            surface: [
                target(device, width, height, "Path tracer surface albedo"),
                target(device, width, height, "Path tracer surface normal"),
                target(device, width, height, "Path tracer surface extra"),
            ],
        }
    }
}

struct Pipelines {
    trace: wgpu::ComputePipeline,
    trace_layout: wgpu::BindGroupLayout,
    temporal: wgpu::ComputePipeline,
    atrous: wgpu::ComputePipeline,
    remodulate: wgpu::ComputePipeline,
    denoise_layout: wgpu::BindGroupLayout,
    composite: wgpu::RenderPipeline,
    composite_layout: wgpu::BindGroupLayout,
}

pub struct PathTracerPass {
    pipelines: Option<Pipelines>,
    targets: Targets,
    trace_params: wgpu::Buffer,
    /// One uniform buffer a denoise dispatch (temporal, three a-trous steps, remodulate): writes made before one
    /// submit all land before it, so one shared buffer would give every dispatch the last step's values.
    denoise_params: Vec<wgpu::Buffer>,
    sampler: wgpu::Sampler,
    width: u32,
    height: u32,
    current: usize,
    frame: u32,
    /// The history is thrown away on the next frame (enabled again, resized, a camera cut).
    reset: bool,
    pub enabled: bool,
    pub bounces: u32,
    /// Ray-traced reflections only (mode 1), when the whole picture is not being traced.
    pub reflections: bool,
    /// The roughest surface that traces a reflection in mode 1, 0..=1 (Post Processing's authored limit).
    pub reflection_roughness: f32,
    /// The mode last recorded: a change throws the history away.
    mode: u32,
}

fn storage(binding: u32) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::COMPUTE,
        ty: wgpu::BindingType::StorageTexture {
            access: wgpu::StorageTextureAccess::WriteOnly,
            format: FORMAT,
            view_dimension: wgpu::TextureViewDimension::D2,
        },
        count: None,
    }
}

fn sampled(binding: u32) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::COMPUTE,
        ty: wgpu::BindingType::Texture {
            sample_type: wgpu::TextureSampleType::Float { filterable: false },
            view_dimension: wgpu::TextureViewDimension::D2,
            multisampled: false,
        },
        count: None,
    }
}

fn uniform(binding: u32) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::COMPUTE,
        ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: false, min_binding_size: None },
        count: None,
    }
}

fn sampler_entry(binding: u32) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::COMPUTE,
        ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
        count: None,
    }
}

impl PathTracerPass {
    /// `hdr_format`: the scene's HDR target, which `composite` draws over.
    pub fn new(
        device: &wgpu::Device,
        shaders: &crate::shaders::Shaders,
        global_layout: &wgpu::BindGroupLayout,
        supported: bool,
        hdr_format: wgpu::TextureFormat,
        width: u32,
        height: u32,
    ) -> Self {
        // The materials' sampler (`default_sampler` in rt_hit.wgsl and the primary's textures): tiling repeats, and
        // anisotropic like the raster's. (Clamped, every tiled texture smeared its edge into streaks, 17 Oct.)
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("Path tracer sampler"),
            address_mode_u: wgpu::AddressMode::Repeat,
            address_mode_v: wgpu::AddressMode::Repeat,
            address_mode_w: wgpu::AddressMode::Repeat,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::MipmapFilterMode::Linear,
            anisotropy_clamp: 16,
            ..Default::default()
        });
        let trace_params = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Path tracer params"),
            size: std::mem::size_of::<PtParams>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let denoise_params = (0..2 + STEPS.len())
            .map(|_| {
                device.create_buffer(&wgpu::BufferDescriptor {
                    label: Some("Path denoise params"),
                    size: std::mem::size_of::<DnParams>() as u64,
                    usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                    mapped_at_creation: false,
                })
            })
            .collect();
        let pipelines = supported.then(|| Self::pipelines(device, shaders, global_layout, hdr_format));
        Self {
            pipelines,
            targets: Targets::new(device, width, height),
            trace_params,
            denoise_params,
            sampler,
            width: width.max(1),
            height: height.max(1),
            current: 0,
            frame: 0,
            reset: true,
            enabled: false,
            bounces: 3,
            reflections: false,
            reflection_roughness: REFLECT_MAX_ROUGH,
            mode: 0,
        }
    }

    fn pipelines(
        device: &wgpu::Device,
        shaders: &crate::shaders::Shaders,
        global_layout: &wgpu::BindGroupLayout,
        hdr_format: wgpu::TextureFormat,
    ) -> Pipelines {
        let trace_module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("path_tracer.wgsl"),
            source: wgpu::ShaderSource::Wgsl(shaders.source_or_panic("path_tracer.wgsl").into()),
        });
        let denoise_module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("path_denoise.wgsl"),
            source: wgpu::ShaderSource::Wgsl(shaders.source_or_panic("path_denoise.wgsl").into()),
        });
        let trace_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("Path tracer BGL"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::AccelerationStructure { vertex_return: false },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::Cube,
                        multisampled: false,
                    },
                    count: None,
                },
                sampler_entry(2),
                sampler_entry(3),
                uniform(4),
                storage(5),
                storage(6),
                storage(7),
                wgpu::BindGroupLayoutEntry {
                    binding: 9,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Uint,
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                sampled(10),
                sampled(11),
                sampled(12),
            ],
        });
        let trace_pl = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("Path tracer PL"),
            bind_group_layouts: &[Some(global_layout), Some(&trace_layout)],
            immediate_size: 0,
        });
        let denoise_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("Path denoise BGL"),
            entries: &[
                uniform(0),
                sampler_entry(1),
                sampled(2),
                sampled(3),
                sampled(4),
                sampled(5),
                sampled(6),
                sampled(7),
                storage(9),
            ],
        });
        let denoise_pl = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("Path denoise PL"),
            bind_group_layouts: &[Some(&denoise_layout)],
            immediate_size: 0,
        });
        let make = |layout: &wgpu::PipelineLayout, module: &wgpu::ShaderModule, entry: &str| {
            device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                label: Some(entry),
                layout: Some(layout),
                module,
                entry_point: Some(entry),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                cache: None,
            })
        };
        // The composite: the remodulated light added over the scene's HDR target, its alpha left as it is.
        let composite_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("Path composite BGL"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 2,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Texture {
                    sample_type: wgpu::TextureSampleType::Float { filterable: false },
                    view_dimension: wgpu::TextureViewDimension::D2,
                    multisampled: false,
                },
                count: None,
            }],
        });
        let composite_pl = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("Path composite PL"),
            bind_group_layouts: &[Some(&composite_layout)],
            immediate_size: 0,
        });
        let composite = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("Path composite"),
            layout: Some(&composite_pl),
            vertex: wgpu::VertexState {
                module: &denoise_module,
                entry_point: Some("composite_vs"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                buffers: &[],
            },
            fragment: Some(wgpu::FragmentState {
                module: &denoise_module,
                entry_point: Some("composite_fs"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: hdr_format,
                    blend: Some(wgpu::BlendState {
                        color: wgpu::BlendComponent {
                            src_factor: wgpu::BlendFactor::One,
                            dst_factor: wgpu::BlendFactor::One,
                            operation: wgpu::BlendOperation::Add,
                        },
                        alpha: wgpu::BlendComponent {
                            src_factor: wgpu::BlendFactor::Zero,
                            dst_factor: wgpu::BlendFactor::One,
                            operation: wgpu::BlendOperation::Add,
                        },
                    }),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview_mask: None,
            cache: None,
        });
        Pipelines {
            trace: make(&trace_pl, &trace_module, "path_trace"),
            trace_layout,
            temporal: make(&denoise_pl, &denoise_module, "temporal"),
            atrous: make(&denoise_pl, &denoise_module, "atrous"),
            remodulate: make(&denoise_pl, &denoise_module, "remodulate"),
            denoise_layout,
            composite,
            composite_layout,
        }
    }

    pub fn supported(&self) -> bool {
        self.pipelines.is_some()
    }

    /// What the shading pass composites while the path tracer runs.
    pub fn output_view(&self) -> &wgpu::TextureView {
        &self.targets.output.view
    }

    /// Where the shading pass writes its finished surface while `enabled` (shading.wgsl `path_surface_*`). New ones
    /// after a resize, like `output_view`.
    pub fn surface_views(&self) -> [&wgpu::TextureView; 3] {
        let [a, b, c] = &self.targets.surface;
        [&a.view, &b.view, &c.view]
    }

    /// True when the targets were made anew: whoever reads `output_view` must take the new one.
    pub fn resize(&mut self, device: &wgpu::Device, width: u32, height: u32) -> bool {
        if width.max(1) == self.width && height.max(1) == self.height {
            return false;
        }
        self.width = width.max(1);
        self.height = height.max(1);
        self.targets = Targets::new(device, self.width, self.height);
        self.reset = true;
        true
    }

    /// Throw the history away (a camera cut, a level loaded).
    pub fn reset_history(&mut self) {
        self.reset = true;
    }

    #[allow(clippy::too_many_arguments)]
    pub fn record(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        global_bind: &wgpu::BindGroup,
        tlas: &wgpu::Tlas,
        env_view: &wgpu::TextureView,
        env_sampler: &wgpu::Sampler,
        velocity_view: &wgpu::TextureView,
        vis_view: &wgpu::TextureView,
        inv_view_proj: glam::Mat4,
        camera_pos: glam::Vec3,
    ) {
        let Some(p) = &self.pipelines else { return };
        if !self.enabled && !self.reflections {
            self.reset = true;
            return;
        }
        let mode = u32::from(!self.enabled);
        let max_rough = REFLECT_ROUGH.unwrap_or(self.reflection_roughness).clamp(0.0, 1.0);
        if mode != std::mem::replace(&mut self.mode, mode) {
            self.reset = true;
        }
        self.frame = self.frame.wrapping_add(1);
        queue.write_buffer(
            &self.trace_params,
            0,
            bytemuck::bytes_of(&PtParams {
                inv_view_proj: inv_view_proj.to_cols_array_2d(),
                camera_pos: camera_pos.to_array(),
                frame: self.frame,
                bounces: self.bounces.clamp(1, 8),
                lamp_count: u32::MAX,
                firefly: 1.0e9,
                env_intensity: 1.0,
                mode,
                max_rough,
                _pad: [0; 2],
            }),
        );
        let t = &self.targets;
        let groups = self.width.div_ceil(8);
        let rows = self.height.div_ceil(8);
        let trace_bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("Path tracer BG"),
            layout: &p.trace_layout,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::AccelerationStructure(tlas) },
                wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::TextureView(env_view) },
                wgpu::BindGroupEntry { binding: 2, resource: wgpu::BindingResource::Sampler(env_sampler) },
                wgpu::BindGroupEntry { binding: 3, resource: wgpu::BindingResource::Sampler(&self.sampler) },
                wgpu::BindGroupEntry { binding: 4, resource: self.trace_params.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 5, resource: wgpu::BindingResource::TextureView(&t.light.view) },
                wgpu::BindGroupEntry { binding: 6, resource: wgpu::BindingResource::TextureView(&t.gbuf.view) },
                wgpu::BindGroupEntry { binding: 7, resource: wgpu::BindingResource::TextureView(&t.albedo.view) },
                wgpu::BindGroupEntry { binding: 9, resource: wgpu::BindingResource::TextureView(vis_view) },
                wgpu::BindGroupEntry { binding: 10, resource: wgpu::BindingResource::TextureView(&t.surface[0].view) },
                wgpu::BindGroupEntry { binding: 11, resource: wgpu::BindingResource::TextureView(&t.surface[1].view) },
                wgpu::BindGroupEntry { binding: 12, resource: wgpu::BindingResource::TextureView(&t.surface[2].view) },
            ],
        });
        {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor { label: Some("Path trace"), timestamp_writes: None });
            pass.set_pipeline(&p.trace);
            pass.set_bind_group(0, global_bind, &[]);
            pass.set_bind_group(1, &trace_bg, &[]);
            pass.dispatch_workgroups(groups, rows, 1);
        }

        let inv_res = [1.0 / self.width as f32, 1.0 / self.height as f32];
        let reset = u32::from(std::mem::take(&mut self.reset));
        let (cur, prev) = (self.current, 1 - self.current);
        // (pipeline, input, history, output, step)
        let mut steps: Vec<(&wgpu::ComputePipeline, &Target, &Target, &Target, u32)> =
            vec![(&p.temporal, &t.light, &t.accum[prev], &t.accum[cur], 1)];
        let mut input = &t.accum[cur];
        for (k, step) in STEPS.iter().enumerate() {
            let output = if k % 2 == 0 { &t.ping } else { &t.pong };
            steps.push((&p.atrous, input, &t.accum[cur], output, *step));
            input = output;
        }
        steps.push((&p.remodulate, input, &t.accum[cur], &t.output, 1));
        for (k, (pipeline, input, history, output, step)) in steps.into_iter().enumerate() {
            queue.write_buffer(
                &self.denoise_params[k],
                0,
                bytemuck::bytes_of(&DnParams {
                    inv_res,
                    step,
                    reset,
                    max_history: if mode == 1 { REFLECT_HISTORY } else { MAX_HISTORY },
                    sigma_light: SIGMA_LIGHT,
                    // Mode 1 carries its roughness limit: 1 + limit (path_denoise.wgsl fades reflections out below it).
                    _pad: [*DEBUG as f32, if mode == 1 { 1.0 + max_rough } else { 0.0 }],
                }),
            );
            let bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("Path denoise BG"),
                layout: &p.denoise_layout,
                entries: &[
                    wgpu::BindGroupEntry { binding: 0, resource: self.denoise_params[k].as_entire_binding() },
                    wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::Sampler(&self.sampler) },
                    wgpu::BindGroupEntry { binding: 2, resource: wgpu::BindingResource::TextureView(&input.view) },
                    wgpu::BindGroupEntry { binding: 3, resource: wgpu::BindingResource::TextureView(&t.gbuf.view) },
                    wgpu::BindGroupEntry { binding: 4, resource: wgpu::BindingResource::TextureView(&t.gbuf_prev.view) },
                    wgpu::BindGroupEntry { binding: 5, resource: wgpu::BindingResource::TextureView(&history.view) },
                    wgpu::BindGroupEntry { binding: 6, resource: wgpu::BindingResource::TextureView(velocity_view) },
                    wgpu::BindGroupEntry { binding: 7, resource: wgpu::BindingResource::TextureView(&t.albedo.view) },
                    wgpu::BindGroupEntry { binding: 9, resource: wgpu::BindingResource::TextureView(&output.view) },
                ],
            });
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor { label: Some("Path denoise"), timestamp_writes: None });
            pass.set_pipeline(pipeline);
            pass.set_bind_group(0, &bg, &[]);
            pass.dispatch_workgroups(groups, rows, 1);
        }
        // This frame's surfaces are next frame's history check; the histories swap.
        encoder.copy_texture_to_texture(
            t.gbuf.texture.as_image_copy(),
            t.gbuf_prev.texture.as_image_copy(),
            wgpu::Extent3d { width: self.width, height: self.height, depth_or_array_layers: 1 },
        );
        self.current = prev;
    }

    /// Mode 0, after `record`: the traced light added over `target`, the scene's HDR picture the shading pass drew
    /// (its direct light, glow, sky and fog). The same size as this pass's targets.
    pub fn composite(&self, device: &wgpu::Device, encoder: &mut wgpu::CommandEncoder, target: &wgpu::TextureView) {
        let Some(p) = &self.pipelines else { return };
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("Path composite BG"),
            layout: &p.composite_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 2,
                resource: wgpu::BindingResource::TextureView(&self.targets.output.view),
            }],
        });
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("Path composite"),
            multiview_mask: None,
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: target,
                resolve_target: None,
                depth_slice: None,
                ops: wgpu::Operations { load: wgpu::LoadOp::Load, store: wgpu::StoreOp::Store },
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
        });
        pass.set_pipeline(&p.composite);
        pass.set_bind_group(0, &bind_group, &[]);
        pass.draw(0..3, 0..1);
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn the_uniforms_match_their_wgsl_layouts() {
        // path_tracer.wgsl PtParams: mat4 (64), vec3 + u32 (16), four 32-bit fields (16), mode, max_rough, vec2 (16).
        assert_eq!(std::mem::size_of::<super::PtParams>(), 112);
        // path_denoise.wgsl DnParams: vec2, two u32, two f32, vec2 of padding.
        assert_eq!(std::mem::size_of::<super::DnParams>(), 32);
    }
}
