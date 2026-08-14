//! Terrain clipmap generate compute (Phase DF).
//!
//! Records before shading. Each dirty rectangle of each ring is one dispatch
//! into the matching storage array. Group 0 is the global pool (bindless
//! sources + `terrain_materials`); group 1 is the destination + params.

use crate::terrain::clipmap::{ClipmapGenJob, GpuClipmapGen, TerrainClipmap};

/// wgpu uniform offset alignment. One slot per dirty rectangle.
const PARAMS_STRIDE: u64 = 256;
const MAX_JOBS: usize = 64;

pub struct TerrainClipmapPass {
    pipeline: wgpu::ComputePipeline,
    layout: wgpu::BindGroupLayout,
    params: wgpu::Buffer,
    sampler: wgpu::Sampler,
}

impl TerrainClipmapPass {
    pub fn new(device: &wgpu::Device, global_layout: &wgpu::BindGroupLayout) -> Self {
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("Terrain clipmap gen BGL"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::StorageTexture {
                        access: wgpu::StorageTextureAccess::WriteOnly,
                        format: wgpu::TextureFormat::Rgba8Unorm,
                        view_dimension: wgpu::TextureViewDimension::D2Array,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::StorageTexture {
                        access: wgpu::StorageTextureAccess::WriteOnly,
                        format: wgpu::TextureFormat::Rgba8Unorm,
                        view_dimension: wgpu::TextureViewDimension::D2Array,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: true,
                        min_binding_size: std::num::NonZeroU64::new(
                            std::mem::size_of::<GpuClipmapGen>() as u64,
                        ),
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 3,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });

        let source = format!(
            "{}\n{}\n{}\n{}",
            include_str!("../shaders/global_pool.wgsl"),
            include_str!("../shaders/hextile.wgsl"),
            include_str!("../shaders/terrain_material.wgsl"),
            include_str!("../shaders/clipmap_gen.wgsl"),
        );
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("Terrain clipmap generate"),
            source: wgpu::ShaderSource::Wgsl(source.into()),
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("Terrain clipmap generate layout"),
            bind_group_layouts: &[Some(global_layout), Some(&layout)],
            immediate_size: 0,
        });
        let pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("Terrain clipmap generate"),
            layout: Some(&pipeline_layout),
            module: &shader,
            entry_point: Some("clipmap_generate"),
            compilation_options: Default::default(),
            cache: None,
        });

        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("Terrain clipmap generate sampler"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::MipmapFilterMode::Linear,
            ..Default::default()
        });

        let params = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Terrain clipmap gen params"),
            size: PARAMS_STRIDE * MAX_JOBS as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        Self {
            pipeline,
            layout,
            params,
            sampler,
        }
    }

    pub fn record(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        global: &wgpu::BindGroup,
        clipmap: &TerrainClipmap,
        terrain_index: u32,
        jobs: &[ClipmapGenJob],
        is_detail: bool,
    ) {
        if jobs.is_empty() {
            return;
        }
        let (albedo, surface) = if is_detail {
            clipmap.detail_storage()
        } else {
            clipmap.macro_storage()
        };
        let bind =
            device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("Terrain clipmap gen"),
                layout: &self.layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: wgpu::BindingResource::TextureView(albedo),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: wgpu::BindingResource::TextureView(surface),
                    },
                    wgpu::BindGroupEntry {
                        binding: 2,
                        resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                            buffer: &self.params,
                            offset: 0,
                            size: std::num::NonZeroU64::new(
                                std::mem::size_of::<GpuClipmapGen>() as u64
                            ),
                        }),
                    },
                    wgpu::BindGroupEntry {
                        binding: 3,
                        resource: wgpu::BindingResource::Sampler(&self.sampler),
                    },
                ],
            });

        let n = jobs.len().min(MAX_JOBS);
        let mut bytes = vec![0u8; PARAMS_STRIDE as usize * n];
        for (i, job) in jobs.iter().take(n).enumerate() {
            let params = GpuClipmapGen::from_job(terrain_index, job);
            let start = i * PARAMS_STRIDE as usize;
            bytes[start..start + std::mem::size_of::<GpuClipmapGen>()]
                .copy_from_slice(bytemuck::bytes_of(&params));
        }
        queue.write_buffer(&self.params, 0, &bytes);

        let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
            label: Some("Terrain clipmap generate"),
            timestamp_writes: None,
        });
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, global, &[]);
        for (i, job) in jobs.iter().take(n).enumerate() {
            pass.set_bind_group(1, &bind, &[(i as u32) * PARAMS_STRIDE as u32]);
            pass.dispatch_workgroups((job.rect.w + 7) / 8, (job.rect.h + 7) / 8, 1);
        }
    }
}
