//! Froxel volumetrics — aerial perspective and fog (Phases 24U and 25I).
//!
//! Builds a 3-D table of "light scattered into the view ray between the camera
//! and this distance, and how much of the surface behind it survives". The
//! shading pass then applies both with one fetch per pixel.
//!
//! ## Reference Architecture
//!
//! - `example_repo/bevy/bevy-main/crates/bevy_pbr/src/atmosphere/aerial_view_lut.wgsl`
//!   — the 3-D LUT layout, log-space storage, half-slice sampling offset, and
//!   the analytic per-segment integration.
//! - `example_repo/bevy/bevy-main/crates/bevy_pbr/src/volumetric_fog/volumetric_fog.wgsl`
//!   — shadow-sampled in-scattering for shafts and the asymmetry parameter.
//!
//! Bevy keeps these as two separate features: a 3-D LUT for aerial perspective
//! and a screen-space march for fog. Somnium folds them into one volume because
//! they are the same integral over the same ray — see the shader's header — and
//! because a second definition of "what the air is made of" is exactly the
//! duplication Phase 25A-2 spent its length removing.

/// Froxels across the screen and in depth.
///
/// Low resolution is the point: in-scattering is smooth, so it is cheap to
/// integrate coarsely and interpolate. 32³ is the size Bevy and Hillaire both
/// use, and it costs 32 slices × 2 steps = 64 samples per column.
const VOLUME_SIZE: wgpu::Extent3d = wgpu::Extent3d {
    width: 32,
    height: 32,
    depth_or_array_layers: 32,
};

/// How far the volume reaches, in metres.
///
/// Past this the last slice is held, so anything further reads as fully
/// fogged. A kilometre covers the default terrain; the sky beyond it is drawn
/// by the atmosphere itself, which already contains the full march.
const DEFAULT_MAX_DISTANCE: f32 = 1200.0;

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct VolumetricParams {
    inv_view_proj: [[f32; 4]; 4],
    camera_pos: [f32; 3],
    max_distance: f32,
    sun_direction: [f32; 3],
    fog_density: f32,
    sun_illuminance: [f32; 3],
    fog_asymmetry: f32,
    fog_height_falloff: f32,
    fog_base_height: f32,
    shafts_enabled: u32,
    _pad: u32,
}

/// Artist-facing settings for the fog medium.
#[derive(Clone, Copy, Debug)]
pub struct FogSettings {
    /// Extinction per metre. 0 leaves pure atmospheric aerial perspective.
    pub density: f32,
    /// Henyey-Greenstein asymmetry; positive scatters forward.
    pub asymmetry: f32,
    /// Metres over which density falls to 1/e above `base_height`.
    pub height_falloff: f32,
    pub base_height: f32,
    /// Shadow-test each step, which is what draws light shafts.
    pub shafts: bool,
}

impl Default for FogSettings {
    fn default() -> Self {
        Self {
            // A thin haze by default: enough that a kilometre of air reads as
            // distance, far short of weather. Aerial perspective from the
            // atmosphere itself is always present regardless.
            density: 0.0008,
            asymmetry: 0.6,
            height_falloff: 120.0,
            base_height: 0.0,
            shafts: true,
        }
    }
}

pub struct VolumetricPass {
    pipeline: wgpu::ComputePipeline,
    layout: wgpu::BindGroupLayout,
    bind_group: Option<wgpu::BindGroup>,
    params: wgpu::Buffer,
    pub view: wgpu::TextureView,
    pub sampler: wgpu::Sampler,
    pub enabled: bool,
    pub fog: FogSettings,
    pub max_distance: f32,
}

impl VolumetricPass {
    pub fn new(device: &wgpu::Device) -> Self {
        let source = format!(
            "{}\n{}",
            include_str!("../shaders/atmosphere.wgsl"),
            include_str!("../shaders/volumetric.wgsl"),
        );
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("volumetric.wgsl"),
            source: wgpu::ShaderSource::Wgsl(source.into()),
        });

        let lut = |binding: u32| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::COMPUTE,
            ty: wgpu::BindingType::Texture {
                sample_type: wgpu::TextureSampleType::Float { filterable: true },
                view_dimension: wgpu::TextureViewDimension::D2,
                multisampled: false,
            },
            count: None,
        };
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("Volumetric BGL"),
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
                lut(1),
                lut(2),
                wgpu::BindGroupLayoutEntry {
                    binding: 3,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 4,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::StorageTexture {
                        access: wgpu::StorageTextureAccess::WriteOnly,
                        format: wgpu::TextureFormat::Rgba16Float,
                        view_dimension: wgpu::TextureViewDimension::D3,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 5,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Storage { read_only: true },
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
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
            ],
        });

        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("Volumetric PL"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("Volumetric Pipeline"),
            layout: Some(&pipeline_layout),
            module: &shader,
            entry_point: Some("main"),
            compilation_options: Default::default(),
            cache: None,
        });

        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("Volumetric Froxels"),
            size: VOLUME_SIZE,
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D3,
            format: wgpu::TextureFormat::Rgba16Float,
            usage: wgpu::TextureUsages::STORAGE_BINDING | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());

        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("Volumetric Sampler"),
            // Clamped on every axis: sampling past the far slice must hold the
            // fully-fogged value rather than wrap around to the camera.
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            address_mode_w: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });

        Self {
            pipeline,
            layout,
            bind_group: None,
            params: device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("Volumetric Params"),
                size: std::mem::size_of::<VolumetricParams>() as u64,
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            }),
            view,
            sampler,
            // `SOMNIUM_VOLUMETRICS=0` switches the whole volume off, which is
            // the A/B both sub-phases are judged by.
            enabled: std::env::var("SOMNIUM_VOLUMETRICS").as_deref() != Ok("0"),
            fog: FogSettings::default(),
            max_distance: DEFAULT_MAX_DISTANCE,
        }
    }

    /// Bind the atmosphere LUTs and shadow atlas. Cheap to call every frame;
    /// only builds the group once.
    #[allow(clippy::too_many_arguments)]
    pub fn ensure_bind_group(
        &mut self,
        device: &wgpu::Device,
        transmittance: &wgpu::TextureView,
        multiscatter: &wgpu::TextureView,
        lut_sampler: &wgpu::Sampler,
        light_buffer: &wgpu::Buffer,
        shadow_atlas: &wgpu::TextureView,
    ) {
        if self.bind_group.is_some() {
            return;
        }
        self.bind_group = Some(device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("Volumetric BG"),
            layout: &self.layout,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: self.params.as_entire_binding() },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(transmittance),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::TextureView(multiscatter),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: wgpu::BindingResource::Sampler(lut_sampler),
                },
                wgpu::BindGroupEntry {
                    binding: 4,
                    resource: wgpu::BindingResource::TextureView(&self.view),
                },
                wgpu::BindGroupEntry { binding: 5, resource: light_buffer.as_entire_binding() },
                wgpu::BindGroupEntry {
                    binding: 6,
                    resource: wgpu::BindingResource::TextureView(shadow_atlas),
                },
            ],
        }));
    }

    /// Distance the volume spans, which shading needs to map depth to a slice.
    pub fn max_distance(&self) -> f32 {
        if self.enabled { self.max_distance } else { 0.0 }
    }

    pub fn record(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        queue: &wgpu::Queue,
        inv_view_proj: glam::Mat4,
        camera_pos: glam::Vec3,
        sun_direction: glam::Vec3,
        sun_illuminance: glam::Vec3,
    ) {
        let Some(bind_group) = self.bind_group.as_ref() else { return };
        if !self.enabled {
            return;
        }

        queue.write_buffer(
            &self.params,
            0,
            bytemuck::bytes_of(&VolumetricParams {
                inv_view_proj: inv_view_proj.to_cols_array_2d(),
                camera_pos: camera_pos.to_array(),
                max_distance: self.max_distance,
                sun_direction: sun_direction.normalize_or_zero().to_array(),
                fog_density: self.fog.density.max(0.0),
                sun_illuminance: sun_illuminance.to_array(),
                fog_asymmetry: self.fog.asymmetry.clamp(-0.95, 0.95),
                fog_height_falloff: self.fog.height_falloff.max(0.0),
                fog_base_height: self.fog.base_height,
                shafts_enabled: u32::from(self.fog.shafts),
                _pad: 0,
            }),
        );

        let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
            label: Some("Volumetric Froxels"),
            timestamp_writes: None,
        });
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, bind_group, &[]);
        // One thread per froxel *column*: each walks its own depth, because the
        // integral along a ray is sequential and sharing the throughput across
        // slices is what makes it one pass instead of 32.
        pass.dispatch_workgroups(VOLUME_SIZE.width.div_ceil(8), VOLUME_SIZE.height.div_ceil(8), 1);
    }
}
