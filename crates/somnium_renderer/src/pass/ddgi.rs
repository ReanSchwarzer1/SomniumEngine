//! Portable SDF-backed dynamic diffuse global illumination (MORROWIND-AB).
//!
//! This pass has no acceleration-structure binding. A camera-relative probe
//! grid traces Somnium's software SDF and stores L2 spherical harmonics for
//! shading. Ray-query ReSTIR GI remains the higher-quality tier.

pub const PROBE_GRID: u32 = 4;
pub const PROBE_COUNT: u32 = PROBE_GRID * PROBE_GRID * PROBE_GRID;
pub const SH_COEFFS: u32 = 9;
const SH_BUFFER_BYTES: u64 = (PROBE_COUNT * SH_COEFFS * 16) as u64;

fn probe_mask(probes: &[u32]) -> u64 {
    probes
        .iter()
        .fold(0_u64, |mask, probe| mask | (1_u64 << probe))
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DdgiConfig {
    pub spacing: f32,
    pub update_budget: u32,
    pub hysteresis: f32,
    pub intensity: f32,
}

impl Default for DdgiConfig {
    fn default() -> Self {
        Self {
            spacing: 2.0,
            update_budget: 8,
            hysteresis: 0.95,
            intensity: 1.0,
        }
    }
}

impl DdgiConfig {
    #[must_use]
    pub fn sanitized(self) -> Self {
        Self {
            spacing: self.spacing.clamp(0.25, 64.0),
            update_budget: self.update_budget.clamp(1, PROBE_COUNT),
            hysteresis: self.hysteresis.clamp(0.0, 0.99),
            intensity: self.intensity.max(0.0),
        }
    }
}

#[derive(Debug, Default)]
pub struct ProbeUpdateScheduler {
    cursor: u32,
    snapped_origin: Option<glam::Vec3>,
}

impl ProbeUpdateScheduler {
    #[must_use]
    pub fn next(&mut self, budget: u32) -> Vec<u32> {
        let budget = budget.clamp(1, PROBE_COUNT);
        let probes = (0..budget)
            .map(|offset| (self.cursor + offset) % PROBE_COUNT)
            .collect();
        self.cursor = (self.cursor + budget) % PROBE_COUNT;
        probes
    }

    /// Returns true only when an established volume crosses a probe-cell edge.
    pub fn set_origin(&mut self, camera: glam::Vec3, spacing: f32) -> bool {
        let spacing = spacing.max(0.25);
        let snapped = (camera / spacing).floor() * spacing;
        let moved = self
            .snapped_origin
            .is_some_and(|previous| previous != snapped);
        self.snapped_origin = Some(snapped);
        if moved {
            self.cursor = 0;
        }
        moved
    }

    #[must_use]
    pub fn origin(&self) -> glam::Vec3 {
        self.snapped_origin.unwrap_or(glam::Vec3::ZERO)
    }

    #[must_use]
    pub fn cursor(&self) -> u32 {
        self.cursor
    }
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct DdgiParams {
    origin: [f32; 3],
    spacing: f32,
    light_dir: [f32; 3],
    intensity: f32,
    light_color: [f32; 3],
    hysteresis: f32,
    update_start: u32,
    update_budget: u32,
    valid_lo: u32,
    valid_hi: u32,
}

pub struct DdgiPass {
    pipeline: wgpu::ComputePipeline,
    layout: wgpu::BindGroupLayout,
    params: wgpu::Buffer,
    sh_buffer: wgpu::Buffer,
    scheduler: ProbeUpdateScheduler,
    config: DdgiConfig,
    enabled: bool,
    valid_probes: u64,
    last_scene_revision: Option<u64>,
    last_light: Option<([f32; 3], [f32; 3])>,
}

impl DdgiPass {
    pub fn new(device: &wgpu::Device, shaders: &crate::shaders::Shaders) -> Self {
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("DDGI BGL"),
            entries: &[
                texture_entry(0, wgpu::TextureViewDimension::D3),
                texture_entry(1, wgpu::TextureViewDimension::Cube),
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
                buffer_entry(3, wgpu::BufferBindingType::Uniform),
                buffer_entry(4, wgpu::BufferBindingType::Storage { read_only: false }),
            ],
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("DDGI PL"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("ddgi.wgsl"),
            source: wgpu::ShaderSource::Wgsl(shaders.source_or_panic("ddgi.wgsl").into()),
        });
        let pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("Portable DDGI"),
            layout: Some(&pipeline_layout),
            module: &module,
            entry_point: Some("update_probes"),
            compilation_options: wgpu::PipelineCompilationOptions::default(),
            cache: None,
        });
        let params = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("DDGI params"),
            size: std::mem::size_of::<DdgiParams>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let sh_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("DDGI L2 SH probes"),
            size: SH_BUFFER_BYTES,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        Self {
            pipeline,
            layout,
            params,
            sh_buffer,
            scheduler: ProbeUpdateScheduler::default(),
            config: DdgiConfig::default(),
            enabled: false,
            valid_probes: 0,
            last_scene_revision: None,
            last_light: None,
        }
    }

    pub fn configure(&mut self, enabled: bool, config: DdgiConfig) {
        let config = config.sanitized();
        if self.config != config || self.enabled != enabled {
            self.valid_probes = 0;
        }
        self.enabled = enabled;
        self.config = config;
    }

    #[must_use]
    pub fn enabled(&self) -> bool {
        self.enabled
    }
    #[must_use]
    pub fn sh_buffer(&self) -> &wgpu::Buffer {
        &self.sh_buffer
    }
    pub fn publish_shading_lattice(&self, lighting_params: &mut [f32; 4]) {
        if self.enabled {
            lighting_params[2] = self.config.spacing;
            lighting_params[3] = PROBE_GRID as f32 * 0.5;
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub fn record(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        sdf_view: &wgpu::TextureView,
        env_view: &wgpu::TextureView,
        env_sampler: &wgpu::Sampler,
        camera_pos: glam::Vec3,
        light_dir: glam::Vec3,
        light_color: glam::Vec3,
        scene_revision: u64,
    ) {
        if !self.enabled {
            self.valid_probes = 0;
            return;
        }
        let moved = self.scheduler.set_origin(camera_pos, self.config.spacing);
        let light = (light_dir.to_array(), light_color.to_array());
        if moved
            || self.last_scene_revision != Some(scene_revision)
            || self.last_light.is_some_and(|last| last != light)
        {
            self.valid_probes = 0;
        }
        self.last_scene_revision = Some(scene_revision);
        self.last_light = Some(light);
        // Invalid probes must be black while the budgeted refresh walks the
        // volume. Leaving old coefficients visible would sample radiance from
        // the previous camera cell/scene for up to eight frames.
        if self.valid_probes == 0 {
            encoder.clear_buffer(&self.sh_buffer, 0, None);
        }
        let probes = self.scheduler.next(self.config.update_budget);
        queue.write_buffer(
            &self.params,
            0,
            bytemuck::bytes_of(&DdgiParams {
                origin: self.scheduler.origin().to_array(),
                spacing: self.config.spacing,
                light_dir: light_dir.normalize_or_zero().to_array(),
                intensity: self.config.intensity,
                light_color: light_color.to_array(),
                hysteresis: self.config.hysteresis,
                update_start: probes[0],
                update_budget: probes.len() as u32,
                valid_lo: self.valid_probes as u32,
                valid_hi: (self.valid_probes >> 32) as u32,
            }),
        );
        let bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("DDGI"),
            layout: &self.layout,
            entries: &[
                entry_view(0, sdf_view),
                entry_view(1, env_view),
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::Sampler(env_sampler),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: self.params.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 4,
                    resource: self.sh_buffer.as_entire_binding(),
                },
            ],
        });
        let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
            label: Some("DDGI"),
            timestamp_writes: None,
        });
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, Some(&bind), &[]);
        pass.dispatch_workgroups(probes.len() as u32, 1, 1);
        drop(pass);
        self.valid_probes |= probe_mask(&probes);
    }
}

fn texture_entry(
    binding: u32,
    view_dimension: wgpu::TextureViewDimension,
) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::COMPUTE,
        ty: wgpu::BindingType::Texture {
            sample_type: wgpu::TextureSampleType::Float { filterable: true },
            view_dimension,
            multisampled: false,
        },
        count: None,
    }
}
fn buffer_entry(binding: u32, ty: wgpu::BufferBindingType) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::COMPUTE,
        ty: wgpu::BindingType::Buffer {
            ty,
            has_dynamic_offset: false,
            min_binding_size: None,
        },
        count: None,
    }
}
fn entry_view<'a>(binding: u32, view: &'a wgpu::TextureView) -> wgpu::BindGroupEntry<'a> {
    wgpu::BindGroupEntry {
        binding,
        resource: wgpu::BindingResource::TextureView(view),
    }
}

#[cfg(test)]
mod tests {
    use super::{
        DdgiConfig, DdgiParams, PROBE_COUNT, ProbeUpdateScheduler, SH_BUFFER_BYTES, probe_mask,
    };
    #[test]
    fn config_clamps_authored_values_to_the_gpu_contract() {
        let config = DdgiConfig {
            spacing: -1.0,
            update_budget: u32::MAX,
            hysteresis: 2.0,
            intensity: -4.0,
        }
        .sanitized();
        assert_eq!(config.spacing, 0.25);
        assert_eq!(config.update_budget, PROBE_COUNT);
        assert_eq!(config.hysteresis, 0.99);
        assert_eq!(config.intensity, 0.0);
    }
    #[test]
    fn scheduler_visits_every_probe_without_starvation() {
        let mut scheduler = ProbeUpdateScheduler::default();
        let mut seen = vec![false; PROBE_COUNT as usize];
        for _ in 0..8 {
            for probe in scheduler.next(8) {
                seen[probe as usize] = true;
            }
        }
        assert!(seen.into_iter().all(|visited| visited));
    }
    #[test]
    fn a_snapped_volume_move_invalidates_probe_history() {
        let mut scheduler = ProbeUpdateScheduler::default();
        assert!(!scheduler.set_origin(glam::Vec3::new(0.1, 0.0, 0.1), 2.0));
        assert!(!scheduler.set_origin(glam::Vec3::new(1.9, 0.0, 0.1), 2.0));
        assert!(scheduler.set_origin(glam::Vec3::new(2.1, 0.0, 0.1), 2.0));
        assert_eq!(scheduler.cursor(), 0);
    }
    #[test]
    fn gpu_contract_is_aligned_and_holds_the_complete_grid() {
        assert_eq!(std::mem::size_of::<DdgiParams>(), 64);
        assert_eq!(SH_BUFFER_BYTES, 64 * 9 * 16);
    }

    #[test]
    fn an_invalidated_volume_marks_only_the_refreshed_probes_valid() {
        let mut scheduler = ProbeUpdateScheduler::default();
        let first = scheduler.next(8);
        assert_eq!(probe_mask(&first).count_ones(), 8);
        assert_eq!(probe_mask(&first) & !0xff, 0);
        let second = scheduler.next(8);
        assert_eq!(probe_mask(&second).count_ones(), 8);
        assert_eq!(probe_mask(&second) & 0xff, 0);
    }
}
