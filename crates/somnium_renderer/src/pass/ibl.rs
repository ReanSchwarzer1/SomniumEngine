//! Phase 19A: image-based lighting — environment cubemap generation.
//!
//! ## Reference Architecture
//!
//! Brian Karis, *Real Shading in Unreal Engine 4* (SIGGRAPH 2013) — the
//! split-sum approximation. This file builds its first half: a prefiltered
//! environment map where mip `i` holds the radiance convolved for roughness
//! `i / (mips - 1)`.
//!
//! The environment is captured from the engine's own procedural sky rather
//! than an HDRI asset, so reflections always agree with the sky the camera
//! sees, and they stay correct when the sun moves.
//!
//! Regeneration is cheap but not free, so it runs at startup and only again
//! when the sun direction or colour actually changes.

/// Cubemap face resolution.
const CUBE_SIZE: u32 = 256;
/// Mip levels; mip `i` is prefiltered for roughness `i / (MIP_COUNT - 1)`.
const MIP_COUNT: u32 = 6;

/// Sky-dome luminance (cd/m²) per lux of sun illuminance.
///
/// Mirrors `somnium_core::light_units::SKY_LUMINANCE_PER_LUX`; duplicated
/// because core depends on the renderer rather than the other way round.
const SKY_LUMINANCE_PER_LUX: f32 = 0.08;

/// Photometric luminance of a linear-RGB light colour (Rec. 709 weights).
fn sun_luminance(color: glam::Vec3) -> f32 {
    color.dot(glam::Vec3::new(0.2126, 0.7152, 0.0722))
}

/// Uniform matching `GenParams` in `ibl_gen.wgsl`.
#[repr(C)]
#[derive(Copy, Clone, bytemuck::Pod, bytemuck::Zeroable)]
struct GenParams {
    face: u32,
    roughness: f32,
    _src_size: f32,
    _pad: f32,
    sun_direction: [f32; 4],
    sun_color: [f32; 4],
}

/// Owns the environment cubemap and the passes that fill it.
pub struct IblPass {
    #[allow(dead_code)]
    cubemap: wgpu::Texture,
    /// Full cube view (all mips) — bound into the shading pass.
    pub cube_view: wgpu::TextureView,
    /// Trilinear sampler for the environment map.
    pub sampler: wgpu::Sampler,

    /// Render targets, indexed `[mip * 6 + face]`.
    face_mip_views: Vec<wgpu::TextureView>,

    sky_pipeline: wgpu::RenderPipeline,
    prefilter_pipeline: wgpu::RenderPipeline,
    sky_bind_group: wgpu::BindGroup,
    prefilter_bind_group: wgpu::BindGroup,
    params_buffer: wgpu::Buffer,

    /// Last sun the cubemap was built for, so we only regenerate on change.
    last_sun: Option<([f32; 3], [f32; 3])>,
}

/// Environment map format — HDR, since the sun disk is far brighter than 1.0.
const ENV_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba16Float;

impl IblPass {
    pub fn new(device: &wgpu::Device) -> Self {
        let cubemap = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("Environment Cubemap"),
            size: wgpu::Extent3d {
                width: CUBE_SIZE,
                height: CUBE_SIZE,
                depth_or_array_layers: 6,
            },
            mip_level_count: MIP_COUNT,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: ENV_FORMAT,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });

        let cube_view = cubemap.create_view(&wgpu::TextureViewDescriptor {
            label: Some("Env Cube View"),
            dimension: Some(wgpu::TextureViewDimension::Cube),
            ..Default::default()
        });

        // Prefilter source: mip 0 only. Reading mip 0 while writing mips 1+
        // needs distinct views or it is a usage conflict. The bind group keeps
        // this view alive, so it is not stored on the struct.
        let mip0_view = cubemap.create_view(&wgpu::TextureViewDescriptor {
            label: Some("Env Cube Mip0"),
            dimension: Some(wgpu::TextureViewDimension::Cube),
            base_mip_level: 0,
            mip_level_count: Some(1),
            ..Default::default()
        });

        let mut face_mip_views = Vec::with_capacity((MIP_COUNT * 6) as usize);
        for mip in 0..MIP_COUNT {
            for face in 0..6 {
                face_mip_views.push(cubemap.create_view(&wgpu::TextureViewDescriptor {
                    label: Some("Env Face Target"),
                    dimension: Some(wgpu::TextureViewDimension::D2),
                    base_mip_level: mip,
                    mip_level_count: Some(1),
                    base_array_layer: face,
                    array_layer_count: Some(1),
                    ..Default::default()
                }));
            }
        }

        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("Env Sampler"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            address_mode_w: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            // Trilinear: the shading pass picks a mip from surface roughness.
            mipmap_filter: wgpu::MipmapFilterMode::Linear,
            ..Default::default()
        });

        let params_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("IBL Gen Params"),
            size: std::mem::size_of::<GenParams>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        // Sky pass: uniform only.
        let sky_bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("IBL Sky BGL"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            }],
        });

        // Prefilter pass: uniform + source cube + sampler.
        let prefilter_bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("IBL Prefilter BGL"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::Cube,
                        multisampled: false,
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

        let sky_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("IBL Sky BG"),
            layout: &sky_bgl,
            entries: &[wgpu::BindGroupEntry { binding: 0, resource: params_buffer.as_entire_binding() }],
        });

        let prefilter_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("IBL Prefilter BG"),
            layout: &prefilter_bgl,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: params_buffer.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::TextureView(&mip0_view) },
                wgpu::BindGroupEntry { binding: 2, resource: wgpu::BindingResource::Sampler(&sampler) },
            ],
        });

        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("IBL Gen Shader"),
            source: wgpu::ShaderSource::Wgsl(include_str!("../shaders/ibl_gen.wgsl").into()),
        });

        let make_pipeline = |label: &str, bgl: &wgpu::BindGroupLayout, entry: &str| {
            let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some(label),
                bind_group_layouts: &[Some(bgl)],
                immediate_size: 0,
            });
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(label),
                layout: Some(&layout),
                multiview_mask: None,
                vertex: wgpu::VertexState {
                    module: &shader,
                    entry_point: Some("vs_main"),
                    buffers: &[],
                    compilation_options: Default::default(),
                },
                fragment: Some(wgpu::FragmentState {
                    module: &shader,
                    entry_point: Some(entry),
                    targets: &[Some(wgpu::ColorTargetState {
                        format: ENV_FORMAT,
                        blend: Some(wgpu::BlendState::REPLACE),
                        write_mask: wgpu::ColorWrites::ALL,
                    })],
                    compilation_options: Default::default(),
                }),
                primitive: wgpu::PrimitiveState::default(),
                depth_stencil: None,
                multisample: wgpu::MultisampleState::default(),
                cache: None,
            })
        };

        let sky_pipeline = make_pipeline("IBL Sky Pipeline", &sky_bgl, "fs_sky");
        let prefilter_pipeline = make_pipeline("IBL Prefilter Pipeline", &prefilter_bgl, "fs_prefilter");

        Self {
            cubemap,
            cube_view,
            sampler,
            face_mip_views,
            sky_pipeline,
            prefilter_pipeline,
            sky_bind_group,
            prefilter_bind_group,
            params_buffer,
            last_sun: None,
        }
    }

    /// Rebuild the cubemap if the sun changed (or it has never been built).
    ///
    /// Returns `true` when work was done.
    pub fn generate_if_needed(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        sun_direction: glam::Vec3,
        sun_color: glam::Vec3,
    ) -> bool {
        let key = (sun_direction.to_array(), sun_color.to_array());
        if self.last_sun == Some(key) {
            return false;
        }
        self.last_sun = Some(key);

        // One submission per face/mip. `queue.write_buffer` only lands once
        // before a submission executes, so a single command buffer would see
        // the last parameters for every pass — 36 tiny submissions is the
        // simple correct alternative, and this runs once, not per frame.
        let dir = sun_direction.normalize_or(glam::Vec3::Y);

        // Mip 0: capture the sky.
        for face in 0..6u32 {
            self.write_params(queue, face, 0.0, dir, sun_color);
            self.run_face(device, queue, 0, face, true);
        }

        // Mips 1..N: GGX prefilter of mip 0, one roughness per level.
        for mip in 1..MIP_COUNT {
            let roughness = mip as f32 / (MIP_COUNT - 1) as f32;
            for face in 0..6u32 {
                self.write_params(queue, face, roughness, dir, sun_color);
                self.run_face(device, queue, mip, face, false);
            }
        }
        true
    }

    fn write_params(
        &self,
        queue: &wgpu::Queue,
        face: u32,
        roughness: f32,
        sun_direction: glam::Vec3,
        sun_color: glam::Vec3,
    ) {
        let p = GenParams {
            face,
            roughness,
            _src_size: CUBE_SIZE as f32,
            _pad: 0.0,
            sun_direction: [sun_direction.x, sun_direction.y, sun_direction.z, 0.0],
            // Phase 24A: `.w` carries the sky-dome luminance scale. The sky
            // gradient is authored as a unit-ish colour, but with the sun now
            // in lux it has to be a luminance too, or ambient is five orders of
            // magnitude too dark and every shadow reads as pure black.
            //
            // A clear day sky is ~8 000 cd/m² under ~100 000 lux of sun, hence
            // 0.08 cd/m² per lux. Scaling the dome by the sun's own output is
            // also what finally lets night happen: lower the sun and the sky
            // darkens with it, instead of holding daylight ambient forever.
            //
            // Interim — Phase 24C computes sky radiance from real atmospheric
            // scattering and this scale factor disappears.
            sun_color: [
                sun_color.x,
                sun_color.y,
                sun_color.z,
                sun_luminance(sun_color) * SKY_LUMINANCE_PER_LUX,
            ],
        };
        queue.write_buffer(&self.params_buffer, 0, bytemuck::bytes_of(&p));
    }

    fn run_face(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        mip: u32,
        face: u32,
        is_sky: bool,
    ) {
        let view = &self.face_mip_views[(mip * 6 + face) as usize];
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("IBL Gen Encoder"),
        });
        {
            let mut rpass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("IBL Gen Pass"),
                multiview_mask: None,
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view,
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
            if is_sky {
                rpass.set_pipeline(&self.sky_pipeline);
                rpass.set_bind_group(0, &self.sky_bind_group, &[]);
            } else {
                rpass.set_pipeline(&self.prefilter_pipeline);
                rpass.set_bind_group(0, &self.prefilter_bind_group, &[]);
            }
            rpass.draw(0..3, 0..1);
        }
        queue.submit(std::iter::once(encoder.finish()));
    }

    /// Highest mip index, for mapping roughness to a mip in the shading pass.
    pub const fn max_mip() -> f32 {
        (MIP_COUNT - 1) as f32
    }
}
