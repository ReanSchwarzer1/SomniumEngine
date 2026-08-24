//! Phase CONTROL-M: volumetric clouds.
//!
//! # Shape
//!
//! Five GPU resources and five pipelines. Three of the pipelines run rarely —
//! the two noise volumes once per process, the weather map only when an
//! authored parameter changes — and two run per frame: the quarter-resolution
//! march and the world-XZ shadow field.
//!
//! # Where it sits in the frame
//!
//! After the shading pass has drawn the sky and the scene into the HDR target,
//! and **before** the water and transparent passes and before TAA. Two
//! consequences, both deliberate:
//!
//! - The clouds land in the HDR buffer that TAA already resolves, so they
//!   inherit the existing jittered-matrix reprojection rather than growing a
//!   private history. Phase 24F's §18 records what a second, naive history
//!   cost the first time; there is no reason to pay it again.
//! - Aerial perspective is applied **inside the march**, to the cloud layer, at
//!   the cloud's own transmittance-weighted depth — not once over the composite
//!   afterwards. §6.3 is explicit that the latter is wrong, and the reason is
//!   simple: the scene behind a cloud is further away than the cloud is.
//!
//! # Default off, and why that is not a hedge
//!
//! `enabled` starts false and stays false until the `.somtime` row exists.
//! The engine is GPU-bound and shading-dominated (CR-A, DOOM-B); a 2 ms pass on
//! a 19.9 ms frame is a 10% tax that has to be argued. Tile binning and the
//! aerial terrain pipeline shipped off for the same reason and said so.

use wgpu::util::DeviceExt;

/// Base shape volume resolution. **Somnium's decision** — see the shader's
/// header for why the commonly quoted figure is not cited as a source.
const BASE_SIZE: u32 = 128;
/// Detail erosion volume resolution.
const DETAIL_SIZE: u32 = 32;
/// Weather-map resolution.
const WEATHER_SIZE: u32 = 512;
/// World-XZ cloud shadow resolution.
const SHADOW_SIZE: u32 = 512;
/// Fraction of the scene resolution the march runs at.
pub const MARCH_DIVISOR: u32 = 4;

/// Everything the sky's authoring surface can change, in renderer terms.
///
/// A plain struct rather than a component: `somnium_renderer` must not know
/// what an ECS is, and `SkyComponent` must not know what a bind group is. The
/// engine layer converts one into the other once per frame, which is also
/// where CONTROL-L's cloud-coverage track gets a chance to override the
/// authored coverage.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CloudSettings {
    /// Fraction of the sky with cloud in it.
    pub coverage: f32,
    /// `0` stratus … `1` cumulonimbus.
    pub cloud_type: f32,
    /// Metres from the ground to the bottom of the layer.
    pub altitude: f32,
    /// Layer thickness in metres.
    pub thickness: f32,
    /// Overall density multiplier. Zero is a clear sky and skips the march.
    pub density: f32,
    /// Wind velocity in metres per second, world XZ.
    pub wind: [f32; 2],
    /// Strength of the high-frequency erosion.
    pub detail_strength: f32,
    /// Extra absorption in precipitating columns.
    pub precipitation: f32,
    /// Metres of world per repeat of the weather map.
    pub weather_scale: f32,
    /// Metres of world per repeat of the base shape volume.
    pub shape_scale: f32,
    /// Ambient contribution from the sky.
    pub ambient: f32,
    /// Henyey–Greenstein forward lobe.
    pub phase_forward: f32,
    /// Henyey–Greenstein backward lobe. Negative.
    pub phase_backward: f32,
    /// Blend between the lobes.
    pub phase_blend: f32,
    /// `0..1` cloud shadow on the ground.
    pub shadow_strength: f32,
    /// Half-extent of the shadow field in metres.
    pub shadow_extent: f32,
    /// Primary march steps.
    pub max_steps: u32,
    /// Light-march steps toward the sun.
    pub light_steps: u32,
    /// Distance in metres at which the march gives up.
    pub max_distance: f32,
    /// Placement seed for the weather field.
    pub seed: u32,
}

impl Default for CloudSettings {
    fn default() -> Self {
        Self {
            coverage: 0.45,
            cloud_type: 0.4,
            altitude: 1500.0,
            thickness: 2200.0,
            density: 1.0,
            wind: [12.0, 4.0],
            detail_strength: 0.6,
            precipitation: 0.0,
            weather_scale: 24_000.0,
            shape_scale: 6_000.0,
            ambient: 1.0,
            phase_forward: 0.8,
            phase_backward: -0.35,
            phase_blend: 0.4,
            shadow_strength: 0.7,
            shadow_extent: 4_000.0,
            max_steps: 48,
            light_steps: 6,
            max_distance: 60_000.0,
            seed: 1337,
        }
    }
}

impl CloudSettings {
    /// The subset the weather map is generated from.
    ///
    /// Compared bit-for-bit so a regeneration happens exactly when one of
    /// these changes, and never on a frame where only the wind moved.
    fn weather_key(self) -> [u32; 4] {
        [
            self.coverage.to_bits(),
            self.cloud_type.to_bits(),
            self.precipitation.to_bits(),
            self.seed,
        ]
    }
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct NoiseParams {
    coverage: f32,
    cloud_type: f32,
    precipitation: f32,
    seed: f32,
    weather_metres: f32,
    _pad0: f32,
    _pad1: f32,
    _pad2: f32,
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct CloudParams {
    inv_view_proj: [[f32; 4]; 4],
    camera_pos: [f32; 3],
    layer_bottom: f32,
    sun_direction: [f32; 3],
    layer_thickness: f32,
    sun_illuminance: [f32; 3],
    density: f32,
    wind_offset: [f32; 2],
    weather_scale: f32,
    shape_scale: f32,
    detail_strength: f32,
    phase_forward: f32,
    phase_backward: f32,
    phase_blend: f32,
    ambient: f32,
    precipitation: f32,
    jitter_enabled: f32,
    frame: f32,
    max_steps: f32,
    light_steps: f32,
    shadow_extent: f32,
    shadow_strength: f32,
    max_distance: f32,
    volumetric_range: f32,
    _pad0: f32,
    _pad1: f32,
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct CompositeParams {
    inv_low_size: [f32; 2],
    low_size: [f32; 2],
    depth_sigma: f32,
    _pad0: f32,
    _pad1: f32,
    _pad2: f32,
}

pub struct CloudPass {
    // ── Authoring ────────────────────────────────────────────────────────────
    /// Off until the `.somtime` row exists. See the module header.
    ///
    /// Written every frame from `SkyComponent::enabled`, except when
    /// [`Self::env_override`] says otherwise.
    pub enabled: bool,
    /// Seam 4: `SOMNIUM_CLOUDS` wins over the authored switch, because every
    /// recorded capture in `dev records/` sets its variables and expects them
    /// to hold. `None` when the variable is unset, which is the ordinary case.
    pub env_override: Option<bool>,
    pub settings: CloudSettings,
    /// Toft & Bowles' blue-noise ray-start offset, behind a switch so the
    /// evidence plan's with-and-without row is a run rather than a rebuild.
    pub jitter: bool,
    /// Depth rejection strength in the upsample. Zero is plain bilinear, which
    /// is the A/B for a suspected halo.
    pub upsample_depth_sigma: f32,

    // ── Resources ────────────────────────────────────────────────────────────
    base_view: wgpu::TextureView,
    detail_view: wgpu::TextureView,
    weather_view: wgpu::TextureView,
    weather_texture: wgpu::Texture,
    scatter_view: wgpu::TextureView,
    /// World-XZ cloud shadow. Public because the shading pass binds it.
    pub shadow_view: wgpu::TextureView,
    /// `[centre_x, centre_z, extent, strength]`, read by `shading.wgsl`.
    pub shadow_params: wgpu::Buffer,
    sampler: wgpu::Sampler,
    cloud_params: wgpu::Buffer,
    noise_params: wgpu::Buffer,
    composite_params: wgpu::Buffer,

    // ── Pipelines ────────────────────────────────────────────────────────────
    noise_bind_group: wgpu::BindGroup,
    base_pipeline: wgpu::ComputePipeline,
    detail_pipeline: wgpu::ComputePipeline,
    weather_pipeline: wgpu::ComputePipeline,

    march_layout: wgpu::BindGroupLayout,
    march_bind_group: Option<wgpu::BindGroup>,
    march_pipeline: wgpu::ComputePipeline,
    shadow_pipeline: wgpu::ComputePipeline,

    composite_layout: wgpu::BindGroupLayout,
    composite_bind_group: Option<wgpu::BindGroup>,
    composite_pipeline: wgpu::RenderPipeline,

    // ── State ────────────────────────────────────────────────────────────────
    /// Noise volumes are generated once, on the first frame the pass runs.
    noise_ready: bool,
    /// The weather key the current map was generated from.
    weather_key: Option<[u32; 4]>,
    /// CPU mirror of the weather map, so a brush can read what is already
    /// there before it writes. `None` until the first paint: an unpainted sky
    /// costs no megabyte.
    weather_cpu: Option<Vec<[u8; 4]>>,
    /// True once a brush has touched the map, which freezes the generator.
    /// A painted field must not be silently overwritten the next time the
    /// coverage slider moves.
    weather_painted: bool,
    /// Wind displacement accumulated in metres, so the sky keeps drifting the
    /// same way across a pause rather than snapping when time resumes.
    wind_offset: [f32; 2],
    frame: u32,
    march_size: (u32, u32),
}

impl CloudPass {
    #[allow(clippy::too_many_lines)]
    pub fn new(device: &wgpu::Device, width: u32, height: u32) -> Self {
        let noise_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("clouds_noise.wgsl"),
            source: wgpu::ShaderSource::Wgsl(include_str!("../shaders/clouds_noise.wgsl").into()),
        });
        // Concatenated after the atmosphere so the clouds read the same LUTs,
        // the same constants and the same sun colour as the sky above them.
        let march_source = format!(
            "{}\n{}",
            include_str!("../shaders/atmosphere.wgsl"),
            include_str!("../shaders/clouds.wgsl"),
        );
        let march_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("clouds.wgsl"),
            source: wgpu::ShaderSource::Wgsl(march_source.into()),
        });
        let composite_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("clouds_composite.wgsl"),
            source: wgpu::ShaderSource::Wgsl(
                include_str!("../shaders/clouds_composite.wgsl").into(),
            ),
        });

        // ── Textures ─────────────────────────────────────────────────────────
        let volume = |label, size: u32| {
            device.create_texture(&wgpu::TextureDescriptor {
                label: Some(label),
                size: wgpu::Extent3d {
                    width: size,
                    height: size,
                    depth_or_array_layers: size,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D3,
                format: wgpu::TextureFormat::Rgba8Unorm,
                usage: wgpu::TextureUsages::STORAGE_BINDING | wgpu::TextureUsages::TEXTURE_BINDING,
                view_formats: &[],
            })
        };
        let base = volume("Cloud Base Noise", BASE_SIZE);
        let detail = volume("Cloud Detail Noise", DETAIL_SIZE);
        let base_view = base.create_view(&wgpu::TextureViewDescriptor::default());
        let detail_view = detail.create_view(&wgpu::TextureViewDescriptor::default());

        let weather = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("Cloud Weather Map"),
            size: wgpu::Extent3d {
                width: WEATHER_SIZE,
                height: WEATHER_SIZE,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::STORAGE_BINDING
                | wgpu::TextureUsages::TEXTURE_BINDING
                // COPY_DST so a brush can paint into the field without the
                // generator having to run — CONTROL-M's painter, and the
                // reason the map is a texture rather than a formula.
                | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        let weather_view = weather.create_view(&wgpu::TextureViewDescriptor::default());
        let weather_texture = weather;

        let shadow = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("Cloud Shadow Field"),
            size: wgpu::Extent3d {
                width: SHADOW_SIZE,
                height: SHADOW_SIZE,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::R16Float,
            usage: wgpu::TextureUsages::STORAGE_BINDING | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });
        let shadow_view = shadow.create_view(&wgpu::TextureViewDescriptor::default());

        let march_size = Self::march_extent(width, height);
        let scatter_view = Self::make_scatter(device, march_size);

        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("Cloud Sampler"),
            address_mode_u: wgpu::AddressMode::Repeat,
            address_mode_v: wgpu::AddressMode::Repeat,
            address_mode_w: wgpu::AddressMode::Repeat,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });

        let uniform = |label, size| {
            device.create_buffer(&wgpu::BufferDescriptor {
                label: Some(label),
                size,
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            })
        };
        let cloud_params = uniform("Cloud Params", std::mem::size_of::<CloudParams>() as u64);
        let noise_params = uniform("Cloud Noise Params", std::mem::size_of::<NoiseParams>() as u64);
        let composite_params = uniform(
            "Cloud Composite Params",
            std::mem::size_of::<CompositeParams>() as u64,
        );
        // Neutral until the first frame: a shading pass that binds this before
        // the cloud pass has ever run must read "no shadow", not "everything
        // is in shadow".
        let shadow_params = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("Cloud Shadow Params"),
            contents: bytemuck::bytes_of(&[0.0_f32, 0.0, 1.0, 0.0]),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });

        // ── Noise pipelines ──────────────────────────────────────────────────
        let storage_3d = |binding| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::COMPUTE,
            ty: wgpu::BindingType::StorageTexture {
                access: wgpu::StorageTextureAccess::WriteOnly,
                format: wgpu::TextureFormat::Rgba8Unorm,
                view_dimension: wgpu::TextureViewDimension::D3,
            },
            count: None,
        };
        let noise_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("Cloud Noise BGL"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                storage_3d(1),
                storage_3d(2),
                wgpu::BindGroupLayoutEntry {
                    binding: 3,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::StorageTexture {
                        access: wgpu::StorageTextureAccess::WriteOnly,
                        format: wgpu::TextureFormat::Rgba8Unorm,
                        view_dimension: wgpu::TextureViewDimension::D2,
                    },
                    count: None,
                },
            ],
        });
        let noise_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("Cloud Noise BG"),
            layout: &noise_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: noise_params.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(&base_view),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::TextureView(&detail_view),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: wgpu::BindingResource::TextureView(&weather_view),
                },
            ],
        });
        let noise_pl = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("Cloud Noise PL"),
            bind_group_layouts: &[Some(&noise_layout)],
            immediate_size: 0,
        });
        let compute = |label: &str, layout: &wgpu::PipelineLayout, entry: &str| {
            device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                label: Some(label),
                layout: Some(layout),
                module: if entry == "march" || entry == "cloud_shadow" {
                    &march_shader
                } else {
                    &noise_shader
                },
                entry_point: Some(entry),
                compilation_options: Default::default(),
                cache: None,
            })
        };
        let base_pipeline = compute("Cloud Base Noise", &noise_pl, "base_noise");
        let detail_pipeline = compute("Cloud Detail Noise", &noise_pl, "detail_noise");
        let weather_pipeline = compute("Cloud Weather Map", &noise_pl, "weather_map");

        // ── March pipelines ──────────────────────────────────────────────────
        let tex_3d = |binding| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::COMPUTE,
            ty: wgpu::BindingType::Texture {
                sample_type: wgpu::TextureSampleType::Float { filterable: true },
                view_dimension: wgpu::TextureViewDimension::D3,
                multisampled: false,
            },
            count: None,
        };
        let tex_2d = |binding| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::COMPUTE,
            ty: wgpu::BindingType::Texture {
                sample_type: wgpu::TextureSampleType::Float { filterable: true },
                view_dimension: wgpu::TextureViewDimension::D2,
                multisampled: false,
            },
            count: None,
        };
        let samp = |binding| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::COMPUTE,
            ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
            count: None,
        };
        let march_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("Cloud March BGL"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                tex_3d(1),
                tex_3d(2),
                tex_2d(3),
                samp(4),
                tex_2d(5),
                tex_2d(6),
                samp(7),
                wgpu::BindGroupLayoutEntry {
                    binding: 8,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::StorageTexture {
                        access: wgpu::StorageTextureAccess::WriteOnly,
                        format: wgpu::TextureFormat::Rgba16Float,
                        view_dimension: wgpu::TextureViewDimension::D2,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 9,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::StorageTexture {
                        access: wgpu::StorageTextureAccess::WriteOnly,
                        format: wgpu::TextureFormat::R16Float,
                        view_dimension: wgpu::TextureViewDimension::D2,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 10,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Depth,
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                tex_3d(11),
            ],
        });
        let march_pl = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("Cloud March PL"),
            bind_group_layouts: &[Some(&march_layout)],
            immediate_size: 0,
        });
        let march_pipeline = compute("Cloud March", &march_pl, "march");
        let shadow_pipeline = compute("Cloud Shadow", &march_pl, "cloud_shadow");

        // ── Composite ────────────────────────────────────────────────────────
        let composite_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("Cloud Composite BGL"),
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
                        view_dimension: wgpu::TextureViewDimension::D2,
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
                wgpu::BindGroupLayoutEntry {
                    binding: 3,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Depth,
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
            ],
        });
        let composite_pl = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("Cloud Composite PL"),
            bind_group_layouts: &[Some(&composite_layout)],
            immediate_size: 0,
        });
        let composite_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("Cloud Composite"),
            layout: Some(&composite_pl),
            vertex: wgpu::VertexState {
                module: &composite_shader,
                entry_point: Some("vs_main"),
                buffers: &[],
                compilation_options: Default::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &composite_shader,
                entry_point: Some("fs_main"),
                targets: &[Some(wgpu::ColorTargetState {
                    format: super::postprocess::HDR_FORMAT,
                    // `dst = scatter + dst * transmittance`, which is the whole
                    // composite. Premultiplied inscatter in RGB and
                    // transmittance in A is what makes it a fixed-function
                    // blend with no destination read.
                    blend: Some(wgpu::BlendState {
                        color: wgpu::BlendComponent {
                            src_factor: wgpu::BlendFactor::One,
                            dst_factor: wgpu::BlendFactor::SrcAlpha,
                            operation: wgpu::BlendOperation::Add,
                        },
                        alpha: wgpu::BlendComponent {
                            src_factor: wgpu::BlendFactor::Zero,
                            dst_factor: wgpu::BlendFactor::One,
                            operation: wgpu::BlendOperation::Add,
                        },
                    }),
                    write_mask: wgpu::ColorWrites::COLOR,
                })],
                compilation_options: Default::default(),
            }),
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview_mask: None,
            cache: None,
        });

        Self {
            // §3: "Turning clouds, weather or decals on by default before the
            // profiler says so" is a declared non-goal of the phase.
            enabled: std::env::var("SOMNIUM_CLOUDS").as_deref() == Ok("1"),
            env_override: match std::env::var("SOMNIUM_CLOUDS").as_deref() {
                Ok("1") => Some(true),
                Ok("0") => Some(false),
                _ => None,
            },
            settings: CloudSettings::default(),
            jitter: std::env::var("SOMNIUM_CLOUD_JITTER").as_deref() != Ok("0"),
            upsample_depth_sigma: 0.002,
            base_view,
            detail_view,
            weather_view,
            weather_texture,
            scatter_view,
            shadow_view,
            shadow_params,
            sampler,
            cloud_params,
            noise_params,
            composite_params,
            noise_bind_group,
            base_pipeline,
            detail_pipeline,
            weather_pipeline,
            march_layout,
            march_bind_group: None,
            march_pipeline,
            shadow_pipeline,
            composite_layout,
            composite_bind_group: None,
            composite_pipeline,
            noise_ready: false,
            weather_key: None,
            weather_cpu: None,
            weather_painted: false,
            wind_offset: [0.0, 0.0],
            frame: 0,
            march_size,
        }
    }

    fn march_extent(width: u32, height: u32) -> (u32, u32) {
        (
            (width / MARCH_DIVISOR).max(1),
            (height / MARCH_DIVISOR).max(1),
        )
    }

    fn make_scatter(device: &wgpu::Device, size: (u32, u32)) -> wgpu::TextureView {
        device
            .create_texture(&wgpu::TextureDescriptor {
                label: Some("Cloud Scatter"),
                size: wgpu::Extent3d {
                    width: size.0,
                    height: size.1,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::Rgba16Float,
                usage: wgpu::TextureUsages::STORAGE_BINDING
                    | wgpu::TextureUsages::TEXTURE_BINDING,
                view_formats: &[],
            })
            .create_view(&wgpu::TextureViewDescriptor::default())
    }

    /// The quarter-resolution buffer the march writes, in texels.
    pub fn march_size(&self) -> (u32, u32) {
        self.march_size
    }

    /// Rebuild the scene-sized resources. Both bind groups are dropped, so the
    /// next `ensure_bind_groups` rebuilds them against the new views.
    pub fn resize(&mut self, device: &wgpu::Device, width: u32, height: u32) {
        let size = Self::march_extent(width, height);
        if size == self.march_size {
            return;
        }
        self.march_size = size;
        self.scatter_view = Self::make_scatter(device, size);
        self.march_bind_group = None;
        self.composite_bind_group = None;
    }

    /// Bind the atmosphere LUTs, the depth buffer and the froxel volume.
    ///
    /// Cheap to call every frame; only builds the groups once per resize.
    pub fn ensure_bind_groups(
        &mut self,
        device: &wgpu::Device,
        transmittance: &wgpu::TextureView,
        multiscatter: &wgpu::TextureView,
        lut_sampler: &wgpu::Sampler,
        depth: &wgpu::TextureView,
        volumetrics: &wgpu::TextureView,
    ) {
        if self.march_bind_group.is_none() {
            self.march_bind_group = Some(device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("Cloud March BG"),
                layout: &self.march_layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: self.cloud_params.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: wgpu::BindingResource::TextureView(&self.base_view),
                    },
                    wgpu::BindGroupEntry {
                        binding: 2,
                        resource: wgpu::BindingResource::TextureView(&self.detail_view),
                    },
                    wgpu::BindGroupEntry {
                        binding: 3,
                        resource: wgpu::BindingResource::TextureView(&self.weather_view),
                    },
                    wgpu::BindGroupEntry {
                        binding: 4,
                        resource: wgpu::BindingResource::Sampler(&self.sampler),
                    },
                    wgpu::BindGroupEntry {
                        binding: 5,
                        resource: wgpu::BindingResource::TextureView(transmittance),
                    },
                    wgpu::BindGroupEntry {
                        binding: 6,
                        resource: wgpu::BindingResource::TextureView(multiscatter),
                    },
                    wgpu::BindGroupEntry {
                        binding: 7,
                        resource: wgpu::BindingResource::Sampler(lut_sampler),
                    },
                    wgpu::BindGroupEntry {
                        binding: 8,
                        resource: wgpu::BindingResource::TextureView(&self.scatter_view),
                    },
                    wgpu::BindGroupEntry {
                        binding: 9,
                        resource: wgpu::BindingResource::TextureView(&self.shadow_view),
                    },
                    wgpu::BindGroupEntry {
                        binding: 10,
                        resource: wgpu::BindingResource::TextureView(depth),
                    },
                    wgpu::BindGroupEntry {
                        binding: 11,
                        resource: wgpu::BindingResource::TextureView(volumetrics),
                    },
                ],
            }));
        }
        if self.composite_bind_group.is_none() {
            self.composite_bind_group =
                Some(device.create_bind_group(&wgpu::BindGroupDescriptor {
                    label: Some("Cloud Composite BG"),
                    layout: &self.composite_layout,
                    entries: &[
                        wgpu::BindGroupEntry {
                            binding: 0,
                            resource: self.composite_params.as_entire_binding(),
                        },
                        wgpu::BindGroupEntry {
                            binding: 1,
                            resource: wgpu::BindingResource::TextureView(&self.scatter_view),
                        },
                        wgpu::BindGroupEntry {
                            binding: 2,
                            resource: wgpu::BindingResource::Sampler(&self.sampler),
                        },
                        wgpu::BindGroupEntry {
                            binding: 3,
                            resource: wgpu::BindingResource::TextureView(depth),
                        },
                    ],
                }));
        }
    }

    /// Advance the wind. Separate from `record` so a paused editor still shows
    /// a still sky rather than one that jumps when time resumes.
    pub fn advance_wind(&mut self, dt: f32) {
        self.wind_offset[0] += self.settings.wind[0] * dt;
        self.wind_offset[1] += self.settings.wind[1] * dt;
        // Wrapped at the weather map's period so the offset cannot grow until
        // it loses float precision — a sky that goes blocky after an hour.
        let period = self.settings.weather_scale.max(1.0);
        self.wind_offset[0] = self.wind_offset[0].rem_euclid(period);
        self.wind_offset[1] = self.wind_offset[1].rem_euclid(period);
    }

    /// `[centre_x, centre_z, extent, strength]` — what `shading.wgsl` needs to
    /// turn a world position into a cloud-shadow lookup.
    fn shadow_uniform(&self, camera_pos: glam::Vec3) -> [f32; 4] {
        shadow_uniform_for(self.enabled, self.settings, camera_pos)
    }

    /// Generate the noise volumes and, when its inputs changed, the weather
    /// map. Called from `record`; separated only for readability.
    fn ensure_fields(&mut self, encoder: &mut wgpu::CommandEncoder, queue: &wgpu::Queue) {
        let key = self.settings.weather_key();
        // A painted map is authored data. Regenerating it because a slider
        // moved would throw away work with no undo, so the generator stands
        // down once a brush has touched the field.
        let regenerate_weather = !self.weather_painted && self.weather_key != Some(key);
        if self.noise_ready && !regenerate_weather {
            return;
        }
        queue.write_buffer(
            &self.noise_params,
            0,
            bytemuck::bytes_of(&NoiseParams {
                coverage: self.settings.coverage.clamp(0.0, 1.0),
                cloud_type: self.settings.cloud_type.clamp(0.0, 1.0),
                precipitation: self.settings.precipitation.clamp(0.0, 1.0),
                #[allow(clippy::cast_precision_loss)]
                seed: (self.settings.seed % 4096) as f32,
                weather_metres: self.settings.weather_scale,
                _pad0: 0.0,
                _pad1: 0.0,
                _pad2: 0.0,
            }),
        );

        let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
            label: Some("Cloud Fields"),
            timestamp_writes: None,
        });
        pass.set_bind_group(0, &self.noise_bind_group, &[]);
        if !self.noise_ready {
            pass.set_pipeline(&self.base_pipeline);
            let g = BASE_SIZE.div_ceil(4);
            pass.dispatch_workgroups(g, g, g);
            pass.set_pipeline(&self.detail_pipeline);
            let g = DETAIL_SIZE.div_ceil(4);
            pass.dispatch_workgroups(g, g, g);
        }
        pass.set_pipeline(&self.weather_pipeline);
        let g = WEATHER_SIZE.div_ceil(8);
        pass.dispatch_workgroups(g, g, 1);
        drop(pass);

        self.noise_ready = true;
        self.weather_key = Some(key);
    }

    /// Which weather-map texel a world XZ position falls in.
    ///
    /// Wrapped, not clamped: the map tiles across the world, so painting near
    /// the seam has to wrap with it or a stroke would pile up at the edge.
    fn weather_texel(&self, world_xz: [f32; 2]) -> [u32; 2] {
        let period = self.settings.weather_scale.max(1.0);
        #[allow(
            clippy::cast_precision_loss,
            clippy::cast_possible_truncation,
            clippy::cast_sign_loss
        )]
        {
            let u = (world_xz[0] + self.wind_offset[0]).rem_euclid(period) / period;
            let v = (world_xz[1] + self.wind_offset[1]).rem_euclid(period) / period;
            [
                ((u * WEATHER_SIZE as f32) as u32).min(WEATHER_SIZE - 1),
                ((v * WEATHER_SIZE as f32) as u32).min(WEATHER_SIZE - 1),
            ]
        }
    }

    /// Stamp a soft brush into one channel of the weather map.
    ///
    /// `channel` is 0 coverage, 1 type, 2 precipitation. `delta` is signed, so
    /// the same call erases with a negative value — one gesture, two
    /// directions, which is how every paint tool in this editor already works.
    /// `radius_metres` is world-space, so a brush covers the same ground
    /// whatever the weather scale is.
    ///
    /// Returns the number of texels touched, which is what a caller needs in
    /// order to tell a no-op stroke from a real one.
    pub fn paint_weather(
        &mut self,
        queue: &wgpu::Queue,
        world_xz: [f32; 2],
        radius_metres: f32,
        channel: usize,
        delta: f32,
    ) -> usize {
        if channel > 2 || radius_metres <= 0.0 {
            return 0;
        }
        let period = self.settings.weather_scale.max(1.0);
        #[allow(clippy::cast_precision_loss)]
        let radius_texels = ((radius_metres / period) * WEATHER_SIZE as f32).max(1.0);
        let centre = self.weather_texel(world_xz);

        let map = self.weather_cpu.get_or_insert_with(|| {
            // The GPU map cannot be read back cheaply, so the first stroke
            // starts from a flat field rather than from the generated one and
            // says so by taking ownership: from here the map is authored.
            vec![[0, 0, 0, 255]; (WEATHER_SIZE * WEATHER_SIZE) as usize]
        });

        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let r = radius_texels.ceil() as i32;
        let mut touched = 0;
        for dy in -r..=r {
            for dx in -r..=r {
                #[allow(clippy::cast_precision_loss)]
                let distance = f32::sqrt((dx * dx + dy * dy) as f32);
                if distance > radius_texels {
                    continue;
                }
                // Raised cosine falloff, so a stroke has no hard rim. A linear
                // edge shows as a ring the moment two strokes overlap.
                let falloff =
                    0.5 * (1.0 + f32::cos(distance / radius_texels * std::f32::consts::PI));
                #[allow(clippy::cast_possible_wrap)]
                let x = (centre[0] as i32 + dx).rem_euclid(WEATHER_SIZE as i32);
                #[allow(clippy::cast_possible_wrap)]
                let y = (centre[1] as i32 + dy).rem_euclid(WEATHER_SIZE as i32);
                #[allow(clippy::cast_sign_loss)]
                let index = (y as usize) * WEATHER_SIZE as usize + x as usize;
                let current = f32::from(map[index][channel]) / 255.0;
                let next = (current + delta * falloff).clamp(0.0, 1.0);
                #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
                {
                    map[index][channel] = (next * 255.0 + 0.5) as u8;
                }
                touched += 1;
            }
        }
        if touched == 0 {
            return 0;
        }
        self.weather_painted = true;

        // Upload the whole map rather than a dirty rect. 512 squared at RGBA8
        // is 1 MB, a stroke that crosses the seam produces two disjoint rects,
        // and one full upload per stroke event is measurably nothing next to
        // the march it feeds.
        let map = self.weather_cpu.as_ref().expect("just inserted");
        queue.write_texture(
            self.weather_texture.as_image_copy(),
            bytemuck::cast_slice(map),
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(WEATHER_SIZE * 4),
                rows_per_image: Some(WEATHER_SIZE),
            },
            wgpu::Extent3d {
                width: WEATHER_SIZE,
                height: WEATHER_SIZE,
                depth_or_array_layers: 1,
            },
        );
        touched
    }

    /// Hand the weather field back to the procedural generator.
    ///
    /// The way out of "I painted the sky and now want the sliders back".
    pub fn clear_painted_weather(&mut self) {
        self.weather_cpu = None;
        self.weather_painted = false;
        self.weather_key = None;
    }

    /// Whether a brush has taken ownership of the weather field.
    pub fn weather_is_painted(&self) -> bool {
        self.weather_painted
    }

    /// March the clouds and rebuild the ground shadow field.
    ///
    /// Writes the shadow uniform unconditionally, including when the pass is
    /// disabled, so the shading pass always reads a truthful "no shadow"
    /// rather than a stale one.
    #[allow(clippy::too_many_arguments)]
    pub fn record(
        &mut self,
        encoder: &mut wgpu::CommandEncoder,
        queue: &wgpu::Queue,
        inv_view_proj: glam::Mat4,
        camera_pos: glam::Vec3,
        sun_direction: glam::Vec3,
        sun_illuminance: glam::Vec3,
        volumetric_range: f32,
    ) {
        queue.write_buffer(
            &self.shadow_params,
            0,
            bytemuck::bytes_of(&self.shadow_uniform(camera_pos)),
        );
        if !self.enabled || self.march_bind_group.is_none() {
            return;
        }
        // Before the bind group is borrowed: `ensure_fields` needs `&mut self`
        // and the borrow checker is right to insist the two do not overlap.
        self.ensure_fields(encoder, queue);

        let s = &self.settings;
        queue.write_buffer(
            &self.cloud_params,
            0,
            bytemuck::bytes_of(&CloudParams {
                inv_view_proj: inv_view_proj.to_cols_array_2d(),
                camera_pos: camera_pos.to_array(),
                layer_bottom: s.altitude.max(1.0),
                sun_direction: sun_direction.normalize_or_zero().to_array(),
                layer_thickness: s.thickness.max(1.0),
                sun_illuminance: sun_illuminance.to_array(),
                density: s.density.max(0.0),
                wind_offset: self.wind_offset,
                weather_scale: s.weather_scale.max(1.0),
                shape_scale: s.shape_scale.max(1.0),
                detail_strength: s.detail_strength.clamp(0.0, 1.0),
                phase_forward: s.phase_forward.clamp(0.0, 0.95),
                phase_backward: s.phase_backward.clamp(-0.95, 0.0),
                phase_blend: s.phase_blend.clamp(0.0, 1.0),
                ambient: s.ambient.max(0.0),
                precipitation: s.precipitation.clamp(0.0, 1.0),
                jitter_enabled: f32::from(u8::from(self.jitter)),
                #[allow(clippy::cast_precision_loss)]
                frame: (self.frame % 4096) as f32,
                #[allow(clippy::cast_precision_loss)]
                max_steps: s.max_steps.clamp(8, 256) as f32,
                #[allow(clippy::cast_precision_loss)]
                light_steps: s.light_steps.clamp(1, 16) as f32,
                shadow_extent: s.shadow_extent.max(1.0),
                shadow_strength: s.shadow_strength.clamp(0.0, 1.0),
                max_distance: s.max_distance.max(1.0),
                volumetric_range,
                _pad0: 0.0,
                _pad1: 0.0,
            }),
        );
        #[allow(clippy::cast_precision_loss)]
        queue.write_buffer(
            &self.composite_params,
            0,
            bytemuck::bytes_of(&CompositeParams {
                inv_low_size: [
                    1.0 / self.march_size.0 as f32,
                    1.0 / self.march_size.1 as f32,
                ],
                low_size: [self.march_size.0 as f32, self.march_size.1 as f32],
                depth_sigma: self.upsample_depth_sigma.max(0.0),
                _pad0: 0.0,
                _pad1: 0.0,
                _pad2: 0.0,
            }),
        );

        let bind_group = self
            .march_bind_group
            .as_ref()
            .expect("checked above");
        let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
            label: Some("Cloud March"),
            timestamp_writes: None,
        });
        pass.set_bind_group(0, bind_group, &[]);
        pass.set_pipeline(&self.march_pipeline);
        pass.dispatch_workgroups(
            self.march_size.0.div_ceil(8),
            self.march_size.1.div_ceil(8),
            1,
        );
        pass.set_pipeline(&self.shadow_pipeline);
        let g = SHADOW_SIZE.div_ceil(8);
        pass.dispatch_workgroups(g, g, 1);
        drop(pass);

        self.frame = self.frame.wrapping_add(1);
    }

    /// Composite the marched buffer over the HDR target.
    ///
    /// Separate from `record` because it must run inside a render pass and
    /// after the shading pass has drawn the sky, while the march wants to be a
    /// compute dispatch that can overlap with whatever precedes it.
    pub fn composite(&self, encoder: &mut wgpu::CommandEncoder, hdr: &wgpu::TextureView) {
        if !self.enabled {
            return;
        }
        let Some(bind_group) = self.composite_bind_group.as_ref() else {
            return;
        };
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("Cloud Composite"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: hdr,
                depth_slice: None,
                resolve_target: None,
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
        pass.set_pipeline(&self.composite_pipeline);
        pass.set_bind_group(0, bind_group, &[]);
        pass.draw(0..3, 0..1);
    }
}

/// `[centre_x, centre_z, extent, strength]`, as a free function so it can be
/// exercised without a GPU device.
fn shadow_uniform_for(enabled: bool, settings: CloudSettings, camera_pos: glam::Vec3) -> [f32; 4] {
    if enabled && settings.shadow_strength > 0.0 {
        [
            camera_pos.x,
            camera_pos.z,
            settings.shadow_extent.max(1.0),
            settings.shadow_strength.clamp(0.0, 1.0),
        ]
    } else {
        // Strength zero is what makes the shading pass ignore the texture
        // entirely, so a disabled cloud pass costs one uniform write.
        [0.0, 0.0, 1.0, 0.0]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn params_keep_rust_and_wgsl_uniform_alignment() {
        assert_eq!(std::mem::size_of::<CloudParams>() % 16, 0);
        assert_eq!(std::mem::size_of::<NoiseParams>() % 16, 0);
        assert_eq!(std::mem::size_of::<CompositeParams>() % 16, 0);
    }

    /// The weather map costs a 512² dispatch, so it must regenerate when its
    /// inputs change and never when only the wind moved.
    #[test]
    fn the_weather_key_tracks_only_what_the_map_is_built_from() {
        let base = CloudSettings::default();
        let mut windy = base;
        windy.wind = [90.0, -3.0];
        assert_eq!(base.weather_key(), windy.weather_key());

        let mut cloudier = base;
        cloudier.coverage += 0.01;
        assert_ne!(base.weather_key(), cloudier.weather_key());

        let mut reseeded = base;
        reseeded.seed += 1;
        assert_ne!(base.weather_key(), reseeded.weather_key());
    }

    /// A disabled pass must publish "no shadow", not a stale field: the
    /// shading pass reads this uniform whether or not the clouds ran, and a
    /// stale strength would leave the ground darkened by clouds that are no
    /// longer being drawn.
    #[test]
    fn the_shadow_uniform_is_neutral_unless_the_pass_is_on() {
        let settings = CloudSettings {
            shadow_strength: 1.0,
            shadow_extent: 3000.0,
            ..CloudSettings::default()
        };
        let camera = glam::Vec3::new(120.0, 8.0, -40.0);

        let on = shadow_uniform_for(true, settings, camera);
        assert_eq!(on, [120.0, -40.0, 3000.0, 1.0]);

        let off = shadow_uniform_for(false, settings, camera);
        assert_eq!(off[3], 0.0, "strength zero is what makes shading ignore it");
        assert!(off[2] > 0.0, "extent must stay positive or the lookup divides by zero");

        let unshadowed = shadow_uniform_for(
            true,
            CloudSettings {
                shadow_strength: 0.0,
                ..settings
            },
            camera,
        );
        assert_eq!(unshadowed[3], 0.0);
    }

    #[test]
    fn the_march_runs_at_a_quarter_of_the_scene() {
        assert_eq!(CloudPass::march_extent(1920, 1080), (480, 270));
        // Never zero, however small the window gets: a zero dispatch is a
        // validation error, not a no-op.
        assert_eq!(CloudPass::march_extent(2, 2), (1, 1));
    }

    /// The brush stamp, exercised without a device.
    ///
    /// `paint_weather` needs a queue only for the upload; the arithmetic that
    /// decides *what* it writes is this, and it is the half that can be wrong
    /// in a way a screenshot does not show.
    fn stamp(map: &mut [[u8; 4]], centre: [u32; 2], radius_texels: f32, channel: usize, delta: f32) -> usize {
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let r = radius_texels.ceil() as i32;
        let mut touched = 0;
        for dy in -r..=r {
            for dx in -r..=r {
                #[allow(clippy::cast_precision_loss)]
                let distance = f32::sqrt((dx * dx + dy * dy) as f32);
                if distance > radius_texels {
                    continue;
                }
                let falloff =
                    0.5 * (1.0 + f32::cos(distance / radius_texels * std::f32::consts::PI));
                #[allow(clippy::cast_possible_wrap)]
                let x = (centre[0] as i32 + dx).rem_euclid(WEATHER_SIZE as i32);
                #[allow(clippy::cast_possible_wrap)]
                let y = (centre[1] as i32 + dy).rem_euclid(WEATHER_SIZE as i32);
                #[allow(clippy::cast_sign_loss)]
                let index = (y as usize) * WEATHER_SIZE as usize + x as usize;
                let current = f32::from(map[index][channel]) / 255.0;
                let next = (current + delta * falloff).clamp(0.0, 1.0);
                #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
                {
                    map[index][channel] = (next * 255.0 + 0.5) as u8;
                }
                touched += 1;
            }
        }
        touched
    }

    #[test]
    fn a_brush_is_strongest_at_its_centre_and_zero_at_its_rim() {
        let mut map = vec![[0_u8, 0, 0, 255]; (WEATHER_SIZE * WEATHER_SIZE) as usize];
        let centre = [100_u32, 100];
        let touched = stamp(&mut map, centre, 8.0, 0, 1.0);
        assert!(touched > 0);

        let at = |x: u32, y: u32| map[(y as usize) * WEATHER_SIZE as usize + x as usize][0];
        assert_eq!(at(100, 100), 255, "the centre takes the full delta");
        assert!(at(104, 100) > 0 && at(104, 100) < 255, "the middle is partial");
        // The rim is exactly zero, which is what stops two overlapping strokes
        // showing a ring where their edges meet.
        assert_eq!(at(108, 100), 0);
        assert_eq!(at(120, 100), 0, "outside the radius is untouched");
    }

    #[test]
    fn a_negative_delta_erases_what_a_positive_one_painted() {
        let mut map = vec![[0_u8, 0, 0, 255]; (WEATHER_SIZE * WEATHER_SIZE) as usize];
        stamp(&mut map, [40, 40], 6.0, 0, 1.0);
        stamp(&mut map, [40, 40], 6.0, 0, -1.0);
        let at = |x: u32, y: u32| map[(y as usize) * WEATHER_SIZE as usize + x as usize][0];
        assert_eq!(at(40, 40), 0, "an equal and opposite stroke returns to zero");
    }

    #[test]
    fn a_brush_only_touches_the_channel_it_was_given() {
        let mut map = vec![[0_u8, 0, 0, 255]; (WEATHER_SIZE * WEATHER_SIZE) as usize];
        stamp(&mut map, [10, 10], 4.0, 2, 1.0);
        let texel = map[10 * WEATHER_SIZE as usize + 10];
        assert_eq!(texel[0], 0, "coverage untouched");
        assert_eq!(texel[1], 0, "type untouched");
        assert_eq!(texel[2], 255, "precipitation painted");
    }

    /// The map tiles across the world, so a stroke at the seam must wrap with
    /// it rather than piling up against an edge that does not exist.
    #[test]
    fn a_stroke_at_the_seam_wraps_instead_of_clamping() {
        let mut map = vec![[0_u8, 0, 0, 255]; (WEATHER_SIZE * WEATHER_SIZE) as usize];
        stamp(&mut map, [0, 0], 4.0, 0, 1.0);
        let at = |x: u32, y: u32| map[(y as usize) * WEATHER_SIZE as usize + x as usize][0];
        assert!(at(0, 0) > 0);
        assert!(
            at(WEATHER_SIZE - 1, 0) > 0,
            "the stroke must appear on the far side of the seam"
        );
        assert!(at(0, WEATHER_SIZE - 1) > 0);
    }

    /// Wind must wrap, or the offset grows until a float cannot represent a
    /// metre and the sky goes blocky after an hour of play.
    #[test]
    fn wind_wraps_at_the_weather_period() {
        let settings = CloudSettings::default();
        let period = settings.weather_scale;
        let mut offset = 0.0_f32;
        for _ in 0..10_000 {
            offset = (offset + settings.wind[0] * 1.0).rem_euclid(period);
        }
        assert!(offset >= 0.0 && offset < period, "offset {offset}");
    }
}
