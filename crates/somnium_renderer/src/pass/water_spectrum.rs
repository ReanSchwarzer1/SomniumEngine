//! Phase IV-F two-cascade deterministic spectral water simulation.

use wgpu::util::DeviceExt;

const PARAM_STRIDE: u64 = 256;
const PARAM_SIZE: usize = 64;
const RECORDS_PER_CASCADE: usize = 24;

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct SpectrumParams {
    dimension: u32,
    stage_size: u32,
    axis: u32,
    input_is_a: u32,
    time: f32,
    delta_time: f32,
    patch_length: f32,
    speed: f32,
    wind_dir: [f32; 2],
    choppy: f32,
    foam_decay: f32,
    foam_threshold: f32,
    _pad0: f32,
    _pad1: [f32; 2],
}

struct Cascade {
    dimension: u32,
    patch_length: f32,
    bind_group: wgpu::BindGroup,
    _displacement: wgpu::Texture,
    displacement_view: wgpu::TextureView,
    gradient: wgpu::Texture,
    gradient_view: wgpu::TextureView,
    gradient_history: wgpu::Texture,
}

/// Two periodic, incommensurate inverse-FFT cascades. Gerstner remains the
/// deterministic CPU/query tier; these textures add the optional cinematic
/// surface detail without forcing GPU readback on gameplay systems.
pub struct WaterSpectrumPass {
    update_pipeline: wgpu::ComputePipeline,
    reverse_pipeline: wgpu::ComputePipeline,
    stage_pipeline: wgpu::ComputePipeline,
    compose_pipeline: wgpu::ComputePipeline,
    params: wgpu::Buffer,
    cascades: [Cascade; 2],
    enabled: bool,
    previous_time: f32,
    phase_time: f32,
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
                storage_buffer(0, true),
                storage_buffer(1, false),
                storage_buffer(2, false),
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
                storage_texture(4),
                storage_texture(5),
                wgpu::BindGroupLayoutEntry {
                    binding: 6,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: false },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
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
        let update_pipeline = pipeline("update_spectrum");
        let reverse_pipeline = pipeline("bit_reverse");
        let stage_pipeline = pipeline("fft_stage");
        let compose_pipeline = pipeline("compose");
        let params = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Water spectrum dispatch params"),
            size: PARAM_STRIDE * (RECORDS_PER_CASCADE * 2) as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let first = make_cascade(device, &layout, &params, 256, 192.0, 0x5A17_0C3Du64);
        let second = make_cascade(device, &layout, &params, 512, 53.0, 0xC041_DA7Au64);
        Self {
            update_pipeline,
            reverse_pipeline,
            stage_pipeline,
            compose_pipeline,
            params,
            cascades: [first, second],
            enabled: std::env::var("SOMNIUM_WATER_SPECTRUM").as_deref() != Ok("0"),
            previous_time: 0.0,
            phase_time: 0.0,
            smoothed_simulation: [0.0; 4],
            simulation_initialized: false,
        }
    }

    pub fn enabled(&self) -> bool {
        self.enabled
    }

    pub fn views(&self) -> [(&wgpu::TextureView, &wgpu::TextureView); 2] {
        [
            (
                &self.cascades[0].displacement_view,
                &self.cascades[0].gradient_view,
            ),
            (
                &self.cascades[1].displacement_view,
                &self.cascades[1].gradient_view,
            ),
        ]
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
        let wind_speed = self.smoothed_simulation[1].clamp(0.1, 40.0);
        let phase_speed = (wind_speed / 11.0).sqrt().clamp(0.2, 2.0);
        self.phase_time += delta_time * phase_speed;
        let choppy = (0.45 + wind_speed * 0.045).clamp(0.45, 1.35);
        let foam_decay = self.smoothed_simulation[2].clamp(0.05, 8.0);
        let foam_threshold = self.smoothed_simulation[3].clamp(0.0, 0.95);
        let mut bytes = vec![0u8; (PARAM_STRIDE as usize) * RECORDS_PER_CASCADE * 2];
        let mut record_count = 0usize;
        for cascade in &self.cascades {
            let mut push = |stage_size: u32, axis: u32, input_is_a: u32| {
                let value = SpectrumParams {
                    dimension: cascade.dimension,
                    stage_size,
                    axis,
                    input_is_a,
                    // Integrated phase time preserves continuity when wind
                    // speed changes; absolute_time * new_speed would jump.
                    time: self.phase_time,
                    delta_time,
                    patch_length: cascade.patch_length,
                    speed: 1.0,
                    wind_dir: [0.944, 0.330],
                    choppy,
                    foam_decay,
                    foam_threshold,
                    _pad0: 0.0,
                    _pad1: [0.0; 2],
                };
                let start = record_count * PARAM_STRIDE as usize;
                bytes[start..start + PARAM_SIZE].copy_from_slice(bytemuck::bytes_of(&value));
                record_count += 1;
            };
            push(0, 0, 1); // spectrum update
            push(0, 0, 1); // two-dimensional bit reverse
            let stages = cascade.dimension.ilog2();
            let mut input_is_a = 0u32; // reverse writes B
            for axis in 0..2 {
                for stage in 1..=stages {
                    push(1 << stage, axis, input_is_a);
                    input_is_a ^= 1;
                }
            }
            debug_assert_eq!(input_is_a, 0, "power-of-two cascades finish in B");
            push(0, 0, 0); // compose from B
        }
        queue.write_buffer(
            &self.params,
            0,
            &bytes[..record_count * PARAM_STRIDE as usize],
        );

        let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
            label: Some("Water spectral cascades"),
            timestamp_writes: None,
        });
        let mut record = 0u32;
        for cascade in &self.cascades {
            let groups = cascade.dimension.div_ceil(8);
            pass.set_pipeline(&self.update_pipeline);
            pass.set_bind_group(0, &cascade.bind_group, &[record * PARAM_STRIDE as u32]);
            pass.dispatch_workgroups(groups, groups, 1);
            record += 1;
            pass.set_pipeline(&self.reverse_pipeline);
            pass.set_bind_group(0, &cascade.bind_group, &[record * PARAM_STRIDE as u32]);
            pass.dispatch_workgroups(groups, groups, 3);
            record += 1;
            pass.set_pipeline(&self.stage_pipeline);
            for _axis in 0..2 {
                for _stage in 1..=cascade.dimension.ilog2() {
                    pass.set_bind_group(0, &cascade.bind_group, &[record * PARAM_STRIDE as u32]);
                    pass.dispatch_workgroups(groups, groups, 3);
                    record += 1;
                }
            }
            pass.set_pipeline(&self.compose_pipeline);
            pass.set_bind_group(0, &cascade.bind_group, &[record * PARAM_STRIDE as u32]);
            pass.dispatch_workgroups(groups, groups, 1);
            record += 1;
        }
        drop(pass);
        for cascade in &self.cascades {
            encoder.copy_texture_to_texture(
                cascade.gradient.as_image_copy(),
                cascade.gradient_history.as_image_copy(),
                cascade.gradient.size(),
            );
        }
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

fn storage_buffer(binding: u32, read_only: bool) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::COMPUTE,
        ty: wgpu::BindingType::Buffer {
            ty: wgpu::BufferBindingType::Storage { read_only },
            has_dynamic_offset: false,
            min_binding_size: None,
        },
        count: None,
    }
}

fn storage_texture(binding: u32) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::COMPUTE,
        ty: wgpu::BindingType::StorageTexture {
            access: wgpu::StorageTextureAccess::WriteOnly,
            format: wgpu::TextureFormat::Rgba16Float,
            view_dimension: wgpu::TextureViewDimension::D2,
        },
        count: None,
    }
}

fn make_cascade(
    device: &wgpu::Device,
    layout: &wgpu::BindGroupLayout,
    params: &wgpu::Buffer,
    dimension: u32,
    patch_length: f32,
    seed: u64,
) -> Cascade {
    let initial = initial_spectrum(dimension, patch_length, seed);
    let initial_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("Water initial wind spectrum"),
        contents: bytemuck::cast_slice(&initial),
        usage: wgpu::BufferUsages::STORAGE,
    });
    let field_bytes = dimension as u64 * dimension as u64 * 3 * 8;
    let make_buffer = |label| {
        device.create_buffer(&wgpu::BufferDescriptor {
            label: Some(label),
            size: field_bytes,
            usage: wgpu::BufferUsages::STORAGE,
            mapped_at_creation: false,
        })
    };
    let spectrum_a = make_buffer("Water FFT ping");
    let spectrum_b = make_buffer("Water FFT pong");
    let make_texture = |label, history: bool| {
        device.create_texture(&wgpu::TextureDescriptor {
            label: Some(label),
            size: wgpu::Extent3d {
                width: dimension,
                height: dimension,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba16Float,
            usage: if history {
                wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST
            } else {
                wgpu::TextureUsages::TEXTURE_BINDING
                    | wgpu::TextureUsages::STORAGE_BINDING
                    | wgpu::TextureUsages::COPY_SRC
            },
            view_formats: &[],
        })
    };
    let displacement = make_texture("Water spectral displacement", false);
    let gradient = make_texture("Water spectral gradient and foam", false);
    let gradient_history = make_texture("Water spectral gradient history", true);
    let displacement_view = displacement.create_view(&Default::default());
    let gradient_view = gradient.create_view(&Default::default());
    let history_view = gradient_history.create_view(&Default::default());
    let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("Water spectrum cascade"),
        layout,
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: initial_buffer.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: spectrum_a.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 2,
                resource: spectrum_b.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 3,
                resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                    buffer: params,
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
                resource: wgpu::BindingResource::TextureView(&history_view),
            },
        ],
    });
    Cascade {
        dimension,
        patch_length,
        bind_group,
        _displacement: displacement,
        displacement_view,
        gradient,
        gradient_view,
        gradient_history,
    }
}

fn initial_spectrum(dimension: u32, patch_length: f32, mut state: u64) -> Vec<[f32; 2]> {
    let count = (dimension * dimension) as usize;
    let mut values = Vec::with_capacity(count);
    let wind = glam::Vec2::new(0.944, 0.330).normalize();
    let mut energy = 0.0f64;
    for y in 0..dimension {
        for x in 0..dimension {
            let sx = if x <= dimension / 2 {
                x as f32
            } else {
                x as f32 - dimension as f32
            };
            let sy = if y <= dimension / 2 {
                y as f32
            } else {
                y as f32 - dimension as f32
            };
            let k = glam::Vec2::new(sx, sy) * std::f32::consts::TAU / patch_length;
            let length = k.length();
            let spectrum = if length < 1.0e-4 {
                0.0
            } else {
                let direction = k / length;
                let alignment = direction.dot(wind).abs().powi(4);
                let large_wave = (-1.0 / (length * 22.0).powi(2)).exp();
                let capillary_cut = (-(length * 0.08).powi(2)).exp();
                large_wave * capillary_cut * (0.08 + 0.92 * alignment) / length.powi(4)
            };
            let g0 = gaussian(&mut state);
            let g1 = gaussian(&mut state);
            let scale = (spectrum * 0.5).sqrt();
            let value = [g0 * scale, g1 * scale];
            energy += f64::from(value[0] * value[0] + value[1] * value[1]);
            values.push(value);
        }
    }
    // Normalize the inverse transform to approximately unit RMS so the ECS
    // amplitude remains a meaningful metre-scale control for both cascades.
    let inverse_scale = (dimension as f64 * dimension as f64) / energy.sqrt().max(1.0e-12);
    for value in &mut values {
        value[0] *= inverse_scale as f32;
        value[1] *= inverse_scale as f32;
    }
    values
}

fn gaussian(state: &mut u64) -> f32 {
    let u1 = uniform(state).max(1.0e-7);
    let u2 = uniform(state);
    (-2.0 * u1.ln()).sqrt() * (std::f32::consts::TAU * u2).cos()
}

fn uniform(state: &mut u64) -> f32 {
    *state ^= *state >> 12;
    *state ^= *state << 25;
    *state ^= *state >> 27;
    let bits = state.wrapping_mul(0x2545_F491_4F6C_DD1D);
    ((bits >> 40) as u32) as f32 / (1u32 << 24) as f32
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deterministic_spectrum_is_finite_and_repeatable() {
        let a = initial_spectrum(32, 53.0, 7);
        let b = initial_spectrum(32, 53.0, 7);
        assert_eq!(a, b);
        assert!(a.iter().flatten().all(|value| value.is_finite()));
        assert!(
            a.iter()
                .skip(1)
                .any(|value| value[0] != 0.0 || value[1] != 0.0)
        );
    }

    #[test]
    fn dispatch_parameter_layout_matches_wgsl() {
        assert_eq!(std::mem::size_of::<SpectrumParams>(), PARAM_SIZE);
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
