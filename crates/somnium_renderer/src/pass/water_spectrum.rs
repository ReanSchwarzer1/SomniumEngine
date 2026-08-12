//! Phase IV-K multi-cascade deterministic spectral ocean simulation.
//!
//! Three independently parameterised JONSWAP/TMA cascades are evolved on the
//! GPU and unpacked into a displacement map, a normal/foam map, and a foam
//! accumulator per cascade. The cascade roster and its numeric parameters match
//! the MIT-licensed GodotOceanWaves reference so the two renderers can be
//! compared frame to frame; see `dev records/phase_IV.md` section 14.

const PARAM_STRIDE: u64 = 256;
const PARAM_SIZE: usize = 80;

/// Every cascade uses the same transform size, which lets one butterfly table
/// and one scratch buffer serve all of them. 1024 is the largest row the
/// shared-memory transform can hold inside WebGPU's 16 KiB workgroup budget.
pub const MAP_SIZE: u32 = 1024;

/// The simulation advances on a fixed tick so foam growth and decay do not
/// change character with frame rate.
const UPDATES_PER_SECOND: f32 = 50.0;
const TICK_SECONDS: f32 = 1.0 / UPDATES_PER_SECOND;
/// A long stall should not be replayed as dozens of catch-up ticks.
const MAX_TICKS_PER_FRAME: u32 = 3;

const WATER_DEPTH_METRES: f32 = 20.0;

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct SpectrumParams {
    map_size: u32,
    stages: u32,
    _pad0: u32,
    _pad1: u32,
    tile_length: [f32; 2],
    depth: f32,
    time: f32,
    alpha: f32,
    peak_frequency: f32,
    wind_speed: f32,
    wind_angle: f32,
    swell: f32,
    detail: f32,
    spread: f32,
    whitecap: f32,
    foam_grow_rate: f32,
    foam_decay_rate: f32,
    seed: [i32; 2],
}

/// Authored parameters for one cascade. These are the values a scene or preset
/// controls; everything else in the pass is derived from them.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct WaveCascadeParams {
    /// Side length of the repeating tile, in metres.
    pub tile_length: f32,
    /// How much of this cascade's horizontal/vertical offset reaches geometry.
    pub displacement_scale: f32,
    /// How much of this cascade's slope reaches the shading normal.
    pub normal_scale: f32,
    pub wind_speed: f32,
    pub wind_direction_degrees: f32,
    /// Distance over which the wind has been blowing, in kilometres.
    pub fetch_kilometres: f32,
    /// Biases energy towards long, ordered swell rather than local wind chop.
    pub swell: f32,
    /// Fades the directional distribution towards isotropic.
    pub spread: f32,
    /// Rolls off waves too short for the transform to resolve.
    pub detail: f32,
    /// Jacobian below which the surface counts as folded over.
    pub whitecap: f32,
    /// Rate at which folded surface turns into persistent foam.
    pub foam_amount: f32,
    pub seed: [i32; 2],
}

impl WaveCascadeParams {
    /// JONSWAP equilibrium range parameter.
    ///
    /// Source: <https://wikiwaves.org/Ocean-Wave_Spectra#JONSWAP_Spectrum>.
    fn alpha(&self) -> f32 {
        let fetch = self.fetch_metres();
        0.076 * (self.wind_speed * self.wind_speed / (fetch * 9.81)).powf(0.22)
    }

    /// Angular frequency carrying the most energy, in radians per second.
    fn peak_angular_frequency(&self) -> f32 {
        let fetch = self.fetch_metres();
        22.0 * (9.81 * 9.81 / (self.wind_speed * fetch)).powf(1.0 / 3.0)
    }

    fn fetch_metres(&self) -> f32 {
        (self.fetch_kilometres * 1000.0).max(1.0)
    }
}

/// The shipped ocean roster. Cascade 0 carries the swell that geometry
/// actually follows, cascade 1 adds a second displaced scale, and cascade 2
/// contributes no displacement at all — only the fine slope and foam detail
/// that would alias badly if it moved vertices.
pub const OCEAN_CASCADES: [WaveCascadeParams; 3] = [
    WaveCascadeParams {
        tile_length: 88.0,
        displacement_scale: 1.0,
        normal_scale: 1.0,
        wind_speed: 10.0,
        wind_direction_degrees: 20.0,
        fetch_kilometres: 150.0,
        swell: 0.8,
        spread: 0.2,
        detail: 1.0,
        whitecap: 0.5,
        foam_amount: 8.0,
        seed: [-4703, 8129],
    },
    WaveCascadeParams {
        tile_length: 57.0,
        displacement_scale: 0.75,
        normal_scale: 1.0,
        wind_speed: 5.0,
        wind_direction_degrees: 15.0,
        fetch_kilometres: 150.0,
        swell: 0.8,
        spread: 0.4,
        detail: 1.0,
        whitecap: 0.5,
        foam_amount: 0.0,
        seed: [6211, -1877],
    },
    WaveCascadeParams {
        tile_length: 16.0,
        displacement_scale: 0.0,
        normal_scale: 0.25,
        wind_speed: 20.0,
        wind_direction_degrees: 20.0,
        fetch_kilometres: 550.0,
        swell: 0.8,
        spread: 0.4,
        detail: 1.0,
        whitecap: 0.25,
        foam_amount: 3.0,
        seed: [-9043, -2551],
    },
];

struct Cascade {
    params: WaveCascadeParams,
    /// Offset so the cascades do not momentarily agree and produce one huge
    /// coherent wave.
    time: f32,
    needs_spectrum: bool,
    _spectrum: wgpu::Buffer,
    bind_group: wgpu::BindGroup,
    _displacement: wgpu::Texture,
    displacement_view: wgpu::TextureView,
    _gradient: wgpu::Texture,
    gradient_view: wgpu::TextureView,
    _foam: wgpu::Texture,
}

/// Three periodic, incommensurate inverse-FFT cascades. Gerstner remains the
/// deterministic CPU/query tier; these textures supply the surface detail
/// without forcing a GPU readback on gameplay systems.
pub struct WaterSpectrumPass {
    generate_pipeline: wgpu::ComputePipeline,
    butterfly_pipeline: wgpu::ComputePipeline,
    modulate_pipeline: wgpu::ComputePipeline,
    fft_pipeline: wgpu::ComputePipeline,
    transpose_pipeline: wgpu::ComputePipeline,
    unpack_pipeline: wgpu::ComputePipeline,
    params: wgpu::Buffer,
    _butterfly: wgpu::Buffer,
    _fft_data: wgpu::Buffer,
    cascades: [Cascade; 3],
    enabled: bool,
    butterfly_ready: bool,
    previous_time: f32,
    tick_accumulator: f32,
    smoothed_simulation: [f32; 4],
    simulation_initialized: bool,
}

impl WaterSpectrumPass {
    pub fn new(device: &wgpu::Device) -> Self {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("Water spectrum shader"),
            source: wgpu::ShaderSource::Wgsl(include_str!("../shaders/water_spectrum.wgsl").into()),
        });
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("Water spectrum layout"),
            entries: &[
                storage_buffer(0),
                storage_buffer(1),
                storage_buffer(2),
                wgpu::BindGroupLayoutEntry {
                    binding: 3,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: true,
                        min_binding_size: std::num::NonZeroU64::new(PARAM_SIZE as u64),
                    },
                    count: None,
                },
                storage_texture(4, wgpu::TextureFormat::Rgba16Float, false),
                storage_texture(5, wgpu::TextureFormat::Rgba16Float, false),
                storage_texture(6, wgpu::TextureFormat::R32Float, true),
            ],
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("Water spectrum pipeline layout"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let pipeline = |entry: &'static str| {
            device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                label: Some(entry),
                layout: Some(&pipeline_layout),
                module: &shader,
                entry_point: Some(entry),
                compilation_options: Default::default(),
                cache: None,
            })
        };

        let params = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Water spectrum dispatch params"),
            size: PARAM_STRIDE * OCEAN_CASCADES.len() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let stages = MAP_SIZE.ilog2();
        let butterfly = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Water FFT butterfly factors"),
            size: u64::from(stages) * u64::from(MAP_SIZE) * 16,
            usage: wgpu::BufferUsages::STORAGE,
            mapped_at_creation: false,
        });
        // Cascades are dispatched one after another inside a single compute
        // pass, so the transform scratch can be shared instead of tripled.
        let fft_data = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Water FFT scratch"),
            size: u64::from(MAP_SIZE) * u64::from(MAP_SIZE) * 4 * 2 * 8,
            usage: wgpu::BufferUsages::STORAGE,
            mapped_at_creation: false,
        });

        let cascades = std::array::from_fn(|index| {
            make_cascade(
                device,
                &layout,
                &params,
                &butterfly,
                &fft_data,
                OCEAN_CASCADES[index],
                // The reference starts each cascade at a different phase so
                // they never line up into one implausible crest.
                120.0 + std::f32::consts::PI * index as f32,
            )
        });

        Self {
            generate_pipeline: pipeline("generate_spectrum"),
            butterfly_pipeline: pipeline("butterfly_precompute"),
            modulate_pipeline: pipeline("modulate"),
            fft_pipeline: pipeline("fft_row"),
            transpose_pipeline: pipeline("transpose"),
            unpack_pipeline: pipeline("unpack"),
            params,
            _butterfly: butterfly,
            _fft_data: fft_data,
            cascades,
            enabled: std::env::var("SOMNIUM_WATER_SPECTRUM").as_deref() != Ok("0"),
            butterfly_ready: false,
            previous_time: 0.0,
            tick_accumulator: 0.0,
            smoothed_simulation: [0.0; 4],
            simulation_initialized: false,
        }
    }

    pub fn enabled(&self) -> bool {
        self.enabled
    }

    /// Per-cascade `(1 / tile_length, displacement_scale, normal_scale)` for
    /// the surface shader.
    pub fn map_scales(&self) -> [[f32; 4]; 3] {
        std::array::from_fn(|index| {
            let params = self.cascades[index].params;
            let inverse = 1.0 / params.tile_length.max(0.001);
            [
                inverse,
                inverse,
                params.displacement_scale,
                params.normal_scale,
            ]
        })
    }

    pub fn views(&self) -> [(&wgpu::TextureView, &wgpu::TextureView); 3] {
        std::array::from_fn(|index| {
            (
                &self.cascades[index].displacement_view,
                &self.cascades[index].gradient_view,
            )
        })
    }

    pub fn record(
        &mut self,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        time: f32,
        simulation: [f32; 4],
    ) -> [f32; 4] {
        if !self.enabled {
            let mut gerstner_only = simulation;
            gerstner_only[0] = 0.0;
            return gerstner_only;
        }
        let delta_time = (time - self.previous_time).clamp(0.0, 0.1);
        self.previous_time = time;
        if !self.simulation_initialized {
            self.smoothed_simulation = simulation;
            self.simulation_initialized = true;
        } else {
            self.smoothed_simulation =
                smooth_controls(self.smoothed_simulation, simulation, delta_time);
        }

        self.tick_accumulator += delta_time;
        let mut ticks = 0u32;
        while self.tick_accumulator >= TICK_SECONDS && ticks < MAX_TICKS_PER_FRAME {
            self.tick_accumulator -= TICK_SECONDS;
            ticks += 1;
        }
        if ticks == 0 {
            return self.smoothed_simulation;
        }
        // Catching up several ticks at once advances phase by the whole
        // interval but only runs one transform, which is what the fixed rate
        // is for.
        let step = TICK_SECONDS * ticks as f32;
        for cascade in &mut self.cascades {
            cascade.time += step;
        }

        let mut bytes = vec![0u8; (PARAM_STRIDE as usize) * self.cascades.len()];
        for (index, cascade) in self.cascades.iter().enumerate() {
            let params = cascade.params;
            // The constants normalize `foam_amount` into a usable range; they
            // come from the reference and are not derived from first
            // principles.
            let foam_grow_rate = step * params.foam_amount * 7.5;
            let foam_decay_rate = step * (10.0_f32 - params.foam_amount).max(0.5) * 1.15;
            let value = SpectrumParams {
                map_size: MAP_SIZE,
                stages: MAP_SIZE.ilog2(),
                _pad0: 0,
                _pad1: 0,
                tile_length: [params.tile_length, params.tile_length],
                depth: WATER_DEPTH_METRES,
                time: cascade.time,
                alpha: params.alpha(),
                peak_frequency: params.peak_angular_frequency(),
                wind_speed: params.wind_speed,
                wind_angle: params.wind_direction_degrees.to_radians(),
                swell: params.swell,
                detail: params.detail,
                spread: params.spread,
                whitecap: params.whitecap,
                foam_grow_rate,
                foam_decay_rate,
                seed: params.seed,
            };
            let start = index * PARAM_STRIDE as usize;
            bytes[start..start + PARAM_SIZE].copy_from_slice(bytemuck::bytes_of(&value));
        }
        queue.write_buffer(&self.params, 0, &bytes);

        let groups = MAP_SIZE.div_ceil(16);
        let tiles = MAP_SIZE.div_ceil(32);
        let stages = MAP_SIZE.ilog2();

        let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
            label: Some("Water spectral cascades"),
            timestamp_writes: None,
        });
        if !self.butterfly_ready {
            pass.set_pipeline(&self.butterfly_pipeline);
            pass.set_bind_group(0, &self.cascades[0].bind_group, &[0]);
            pass.dispatch_workgroups(MAP_SIZE / 2 / 64, stages, 1);
            self.butterfly_ready = true;
        }
        for (index, cascade) in self.cascades.iter_mut().enumerate() {
            let offset = (index as u64 * PARAM_STRIDE) as u32;
            if cascade.needs_spectrum {
                pass.set_pipeline(&self.generate_pipeline);
                pass.set_bind_group(0, &cascade.bind_group, &[offset]);
                pass.dispatch_workgroups(groups, groups, 1);
                cascade.needs_spectrum = false;
            }
            pass.set_pipeline(&self.modulate_pipeline);
            pass.set_bind_group(0, &cascade.bind_group, &[offset]);
            pass.dispatch_workgroups(groups, groups, 1);

            pass.set_pipeline(&self.fft_pipeline);
            pass.set_bind_group(0, &cascade.bind_group, &[offset]);
            pass.dispatch_workgroups(1, MAP_SIZE, 4);

            pass.set_pipeline(&self.transpose_pipeline);
            pass.set_bind_group(0, &cascade.bind_group, &[offset]);
            pass.dispatch_workgroups(tiles, tiles, 4);

            pass.set_pipeline(&self.fft_pipeline);
            pass.set_bind_group(0, &cascade.bind_group, &[offset]);
            pass.dispatch_workgroups(1, MAP_SIZE, 4);

            pass.set_pipeline(&self.unpack_pipeline);
            pass.set_bind_group(0, &cascade.bind_group, &[offset]);
            pass.dispatch_workgroups(groups, groups, 1);
        }
        drop(pass);
        self.smoothed_simulation
    }
}

fn smooth_controls(current: [f32; 4], target: [f32; 4], delta_time: f32) -> [f32; 4] {
    let response = 1.0 - (-delta_time.max(0.0) / 1.25).exp();
    let mut result = current;
    for (value, target) in result.iter_mut().zip(target) {
        *value += (target - *value) * response;
    }
    result
}

fn storage_buffer(binding: u32) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::COMPUTE,
        ty: wgpu::BindingType::Buffer {
            ty: wgpu::BufferBindingType::Storage { read_only: false },
            has_dynamic_offset: false,
            min_binding_size: None,
        },
        count: None,
    }
}

fn storage_texture(
    binding: u32,
    format: wgpu::TextureFormat,
    read_write: bool,
) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::COMPUTE,
        ty: wgpu::BindingType::StorageTexture {
            access: if read_write {
                wgpu::StorageTextureAccess::ReadWrite
            } else {
                wgpu::StorageTextureAccess::WriteOnly
            },
            format,
            view_dimension: wgpu::TextureViewDimension::D2,
        },
        count: None,
    }
}

fn make_cascade(
    device: &wgpu::Device,
    layout: &wgpu::BindGroupLayout,
    params_buffer: &wgpu::Buffer,
    butterfly: &wgpu::Buffer,
    fft_data: &wgpu::Buffer,
    params: WaveCascadeParams,
    time: f32,
) -> Cascade {
    let spectrum = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("Water initial wind spectrum"),
        size: u64::from(MAP_SIZE) * u64::from(MAP_SIZE) * 16,
        usage: wgpu::BufferUsages::STORAGE,
        mapped_at_creation: false,
    });
    let make_texture = |label, format| {
        device.create_texture(&wgpu::TextureDescriptor {
            label: Some(label),
            size: wgpu::Extent3d {
                width: MAP_SIZE,
                height: MAP_SIZE,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::STORAGE_BINDING,
            view_formats: &[],
        })
    };
    let displacement = make_texture(
        "Water spectral displacement",
        wgpu::TextureFormat::Rgba16Float,
    );
    let gradient = make_texture(
        "Water spectral gradient and foam",
        wgpu::TextureFormat::Rgba16Float,
    );
    let foam = make_texture("Water foam accumulator", wgpu::TextureFormat::R32Float);
    let displacement_view = displacement.create_view(&Default::default());
    let gradient_view = gradient.create_view(&Default::default());
    let foam_view = foam.create_view(&Default::default());
    let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("Water spectrum cascade"),
        layout,
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: spectrum.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: fft_data.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 2,
                resource: butterfly.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 3,
                resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                    buffer: params_buffer,
                    offset: 0,
                    size: std::num::NonZeroU64::new(PARAM_SIZE as u64),
                }),
            },
            wgpu::BindGroupEntry {
                binding: 4,
                resource: wgpu::BindingResource::TextureView(&displacement_view),
            },
            wgpu::BindGroupEntry {
                binding: 5,
                resource: wgpu::BindingResource::TextureView(&gradient_view),
            },
            wgpu::BindGroupEntry {
                binding: 6,
                resource: wgpu::BindingResource::TextureView(&foam_view),
            },
        ],
    });
    Cascade {
        params,
        time,
        needs_spectrum: true,
        _spectrum: spectrum,
        bind_group,
        _displacement: displacement,
        displacement_view,
        _gradient: gradient,
        gradient_view,
        _foam: foam,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dispatch_parameter_layout_matches_wgsl() {
        assert_eq!(std::mem::size_of::<SpectrumParams>(), PARAM_SIZE);
        // Uniform structs round up to a sixteen-byte multiple.
        assert_eq!(PARAM_SIZE % 16, 0);
    }

    #[test]
    fn jonswap_energy_falls_as_fetch_and_wind_grow() {
        let calm = WaveCascadeParams {
            wind_speed: 5.0,
            ..OCEAN_CASCADES[0]
        };
        let storm = WaveCascadeParams {
            wind_speed: 20.0,
            ..OCEAN_CASCADES[0]
        };
        // A longer, stronger wind field peaks at a lower frequency: bigger,
        // slower waves rather than more of the same chop.
        assert!(storm.peak_angular_frequency() < calm.peak_angular_frequency());
        assert!(storm.alpha() > calm.alpha());
        assert!(calm.alpha().is_finite() && storm.alpha().is_finite());
    }

    #[test]
    fn shipped_cascades_are_incommensurate_and_ordered() {
        let lengths: Vec<f32> = OCEAN_CASCADES.iter().map(|c| c.tile_length).collect();
        assert_eq!(lengths, vec![88.0, 57.0, 16.0]);
        // Repeating tiles that share a factor would visibly line up.
        for window in lengths.windows(2) {
            let ratio = window[0] / window[1];
            assert!((ratio - ratio.round()).abs() > 0.1, "tiles {window:?} align");
        }
        // The finest cascade must not move geometry; a 16 m tile displaced on
        // a metre-scale mesh aliases badly.
        assert_eq!(OCEAN_CASCADES[2].displacement_scale, 0.0);
    }

    #[test]
    fn map_size_row_fits_the_workgroup_storage_budget() {
        let bytes = 2 * MAP_SIZE as usize * std::mem::size_of::<[f32; 2]>();
        assert!(bytes <= 16384, "{bytes} exceeds the guaranteed 16 KiB");
        assert_eq!(MAP_SIZE % 256, 0, "row must divide across 256 invocations");
    }

    #[test]
    fn calm_to_storm_controls_approach_without_a_jump_or_overshoot() {
        let calm = [0.25, 2.0, 0.4, 0.5];
        let storm = [1.0, 24.0, 2.0, 0.04];
        let first = smooth_controls(calm, storm, 1.0 / 60.0);
        assert!(first[0] > calm[0] && first[0] < storm[0]);
        assert!(first[1] > calm[1] && first[1] < storm[1]);
        assert!(first[3] < calm[3] && first[3] > storm[3]);
        let second = smooth_controls(first, storm, 1.0 / 60.0);
        assert!(second[1] > first[1] && second[1] < storm[1]);
    }
}
