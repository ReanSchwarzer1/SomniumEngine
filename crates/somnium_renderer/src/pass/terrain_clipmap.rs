//! Terrain clipmap generate (Phase DF).
//!
//! Fragment pass, recorded before shading. Each dirty rectangle of each ring is
//! one draw into that array layer (color attachments). Group 0 is the global
//! pool; group 1 is params + sampler. Compute storage writes sampled as black
//! on this Vulkan path (Dbg 32).

use crate::terrain::clipmap::{ClipmapGenJob, GpuClipmapGen, TerrainClipmap};

/// wgpu uniform offset alignment. One slot per dirty rectangle.
const PARAMS_STRIDE: u64 = 256;
const MAX_JOBS: usize = 64;

pub struct TerrainClipmapPass {
    pipeline: wgpu::RenderPipeline,
    _layout: wgpu::BindGroupLayout,
    params: wgpu::Buffer,
    _sampler: wgpu::Sampler,
    bind: wgpu::BindGroup,
}

impl TerrainClipmapPass {
    pub fn new(device: &wgpu::Device, global_layout: &wgpu::BindGroupLayout) -> Self {
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("Terrain clipmap gen BGL"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
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
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
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
        let target = Some(wgpu::ColorTargetState {
            format: wgpu::TextureFormat::Rgba8Unorm,
            blend: Some(wgpu::BlendState::REPLACE),
            write_mask: wgpu::ColorWrites::ALL,
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("Terrain clipmap generate"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("clipmap_vs"),
                buffers: &[],
                compilation_options: Default::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("clipmap_generate"),
                targets: &[target.clone(), target],
                compilation_options: Default::default(),
            }),
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                strip_index_format: None,
                front_face: wgpu::FrontFace::Ccw,
                cull_mode: None,
                polygon_mode: wgpu::PolygonMode::Fill,
                unclipped_depth: false,
                conservative: false,
            },
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview_mask: None,
            cache: None,
        });

        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("Terrain clipmap generate sampler"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::MipmapFilterMode::Linear,
            address_mode_u: wgpu::AddressMode::Repeat,
            address_mode_v: wgpu::AddressMode::Repeat,
            address_mode_w: wgpu::AddressMode::Repeat,
            ..Default::default()
        });

        let params = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Terrain clipmap gen params"),
            size: PARAMS_STRIDE * MAX_JOBS as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let bind =
            device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("Terrain clipmap gen"),
                layout: &layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                            buffer: &params,
                            offset: 0,
                            size: std::num::NonZeroU64::new(
                                std::mem::size_of::<GpuClipmapGen>() as u64
                            ),
                        }),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: wgpu::BindingResource::Sampler(&sampler),
                    },
                ],
            });

        Self {
            pipeline,
            _layout: layout,
            params,
            _sampler: sampler,
            bind,
        }
    }

    pub fn record(
        &self,
        _device: &wgpu::Device,
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
        let n = jobs.len().min(MAX_JOBS);
        let mut bytes = vec![0u8; PARAMS_STRIDE as usize * n];
        for (i, job) in jobs.iter().take(n).enumerate() {
            let params = GpuClipmapGen::from_job(terrain_index, job);
            let start = i * PARAMS_STRIDE as usize;
            bytes[start..start + std::mem::size_of::<GpuClipmapGen>()]
                .copy_from_slice(bytemuck::bytes_of(&params));
        }
        queue.write_buffer(&self.params, 0, &bytes);

        // One render pass per **ring**, not per rectangle.
        //
        // `take_jobs` walks the generate order ring by ring, so every job for a
        // ring arrives consecutively and they all target the same pair of array
        // layers. Opening a render pass per rectangle meant a begin/end — and
        // the barrier either side of it — for each arm of an L-shaped slide,
        // which is up to four per ring and up to 32 per frame, to write a few
        // thousand texels. Grouping the run costs nothing and issues one draw
        // per rectangle inside a single pass.
        let load = wgpu::Operations {
            load: wgpu::LoadOp::Load,
            store: wgpu::StoreOp::Store,
        };
        let mut start = 0usize;
        while start < n {
            let ring = jobs[start].ring;
            let mut end = start + 1;
            while end < n && jobs[end].ring == ring {
                end += 1;
            }
            let run = start..end;
            start = end;

            if jobs[run.clone()].iter().all(|job| job.rect.is_empty()) {
                continue;
            }
            let (albedo, surface) = if is_detail {
                clipmap.detail_layer(ring)
            } else {
                clipmap.macro_layer(ring)
            };
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("Terrain clipmap generate"),
                color_attachments: &[
                    Some(wgpu::RenderPassColorAttachment {
                        view: albedo,
                        resolve_target: None,
                        depth_slice: None,
                        ops: load,
                    }),
                    Some(wgpu::RenderPassColorAttachment {
                        view: surface,
                        resolve_target: None,
                        depth_slice: None,
                        ops: load,
                    }),
                ],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pass.set_pipeline(&self.pipeline);
            pass.set_bind_group(0, global, &[]);
            for i in run {
                let job = &jobs[i];
                if job.rect.is_empty() {
                    continue;
                }
                pass.set_bind_group(1, &self.bind, &[(i as u32) * PARAMS_STRIDE as u32]);
                pass.set_viewport(
                    job.rect.x as f32,
                    job.rect.y as f32,
                    job.rect.w as f32,
                    job.rect.h as f32,
                    0.0,
                    1.0,
                );
                pass.set_scissor_rect(job.rect.x, job.rect.y, job.rect.w, job.rect.h);
                pass.draw(0..3, 0..1);
            }
        }
    }
}
