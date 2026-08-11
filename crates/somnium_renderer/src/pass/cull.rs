//! Phase 15B: GPU instance-culling compute pass.
//!
//! Dispatched before the visibility pass. Reads the instance buffer and a
//! parallel array of local AABBs, and writes each draw's verdict directly into
//! the indirect arguments produced by Phase 15A.
//!
//! Requires the GPU-driven path — with the CPU fallback there are no indirect
//! arguments to write into, so culling is simply skipped there.

use crate::culling::{GpuCullAabb, GpuCullParams};

/// Workgroup size; must match `@workgroup_size` in `cull.wgsl`.
const WORKGROUP_SIZE: u32 = 64;

/// Compute pass that flags off-screen draws by zeroing their instance count.
pub struct CullPass {
    pipeline: wgpu::ComputePipeline,
    layout: wgpu::BindGroupLayout,
    /// Per-draw local AABBs, rebuilt each frame alongside the indirect args.
    aabb_buffer: wgpu::Buffer,
    /// Frustum planes, draw count and Hi-Z parameters — one per phase, because
    /// both dispatches are encoded before either runs and a single uniform
    /// buffer could not hold two different `phase` values.
    params_buffers: [wgpu::Buffer; 2],
    /// Phase 15E2: per-draw record of what phase one rejected on occlusion.
    flags_buffer: wgpu::Buffer,
    /// Capacity in draws.
    capacity: usize,
    /// CPU staging for the AABB array, reused across frames.
    staging: Vec<GpuCullAabb>,
}

/// Starting capacity in draws (matches the indirect buffer's).
const INITIAL_CAPACITY: usize = 1024;

impl CullPass {
    pub fn new(device: &wgpu::Device) -> Self {
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("Cull Bind Group Layout"),
            entries: &[
                // 0: instances (read)
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Storage { read_only: true },
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                // 1: local AABBs (read)
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Storage { read_only: true },
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                // 2: indirect draw args (read_write)
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Storage { read_only: false },
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                // 3: cull params (uniform)
                wgpu::BindGroupLayoutEntry {
                    binding: 3,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                // 4: Hi-Z pyramid (read). Sampled with textureLoad only, so it
                // never needs to filter.
                wgpu::BindGroupLayoutEntry {
                    binding: 4,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: false },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                // 5: occlusion-reject flags (read_write)
                wgpu::BindGroupLayoutEntry {
                    binding: 5,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Storage { read_only: false },
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
            ],
        });

        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("Cull Shader"),
            source: wgpu::ShaderSource::Wgsl(include_str!("../shaders/cull.wgsl").into()),
        });

        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("Cull Pipeline Layout"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });

        let pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("Cull Pipeline"),
            layout: Some(&pipeline_layout),
            module: &shader,
            entry_point: Some("cs_main"),
            compilation_options: Default::default(),
            cache: None,
        });

        Self {
            pipeline,
            layout,
            aabb_buffer: Self::alloc_aabbs(device, INITIAL_CAPACITY),
            params_buffers: std::array::from_fn(|phase| {
                device.create_buffer(&wgpu::BufferDescriptor {
                    label: Some(if phase == 0 {
                        "Cull Params P1"
                    } else {
                        "Cull Params P2"
                    }),
                    size: std::mem::size_of::<GpuCullParams>() as u64,
                    usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                    mapped_at_creation: false,
                })
            }),
            flags_buffer: Self::alloc_flags(device, INITIAL_CAPACITY),
            capacity: INITIAL_CAPACITY,
            staging: Vec::with_capacity(INITIAL_CAPACITY),
        }
    }

    fn alloc_flags(device: &wgpu::Device, capacity: usize) -> wgpu::Buffer {
        device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Cull Occlusion Flags"),
            // Phase one writes every entry it reads, so this never needs
            // clearing between frames.
            size: (capacity * std::mem::size_of::<u32>()) as u64,
            usage: wgpu::BufferUsages::STORAGE,
            mapped_at_creation: false,
        })
    }

    fn alloc_aabbs(device: &wgpu::Device, capacity: usize) -> wgpu::Buffer {
        device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Cull AABBs"),
            size: (capacity * std::mem::size_of::<GpuCullAabb>()) as u64,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        })
    }

    /// Upload this frame's AABBs and frustum planes.
    ///
    /// `aabbs` must be parallel to the indirect argument array — entry `i`
    /// bounds draw `i`.
    #[allow(clippy::too_many_arguments)]
    pub fn update(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        aabbs: &[GpuCullAabb],
        view_proj: glam::Mat4,
        disabled: bool,
        hiz_size: (u32, u32),
        hiz_mip_count: u32,
        occlusion_enabled: bool,
        camera_pos: glam::Vec3,
    ) {
        self.staging.clear();
        self.staging.extend_from_slice(aabbs);

        if self.staging.len() > self.capacity {
            let mut cap = self.capacity.max(1);
            while cap < self.staging.len() {
                cap *= 2;
            }
            self.aabb_buffer = Self::alloc_aabbs(device, cap);
            self.flags_buffer = Self::alloc_flags(device, cap);
            self.capacity = cap;
        }

        if !self.staging.is_empty() {
            queue.write_buffer(&self.aabb_buffer, 0, bytemuck::cast_slice(&self.staging));
        }

        let mut params = GpuCullParams {
            planes: crate::culling::frustum_planes(view_proj),
            draw_count: self.staging.len() as u32,
            disabled: u32::from(disabled),
            phase: 0,
            occlusion_enabled: u32::from(occlusion_enabled),
            view_proj: view_proj.to_cols_array_2d(),
            hiz_size: [hiz_size.0 as f32, hiz_size.1 as f32],
            hiz_mip_count,
            _pad: 0,
            camera_pos: [camera_pos.x, camera_pos.y, camera_pos.z, 0.0],
        };
        queue.write_buffer(&self.params_buffers[0], 0, bytemuck::bytes_of(&params));
        params.phase = 1;
        queue.write_buffer(&self.params_buffers[1], 0, bytemuck::bytes_of(&params));
    }

    /// Dispatch one phase of the cull, writing verdicts into `indirect_buffer`.
    ///
    /// `phase` is 0 or 1; see the note above `cs_main` in `cull.wgsl`.
    #[allow(clippy::too_many_arguments)]
    pub fn record(
        &self,
        device: &wgpu::Device,
        encoder: &mut wgpu::CommandEncoder,
        instance_buffer: &wgpu::Buffer,
        indirect_buffer: &wgpu::Buffer,
        hiz_view: &wgpu::TextureView,
        phase: usize,
        draw_count: usize,
    ) {
        if draw_count == 0 {
            return;
        }

        // Rebuilt per frame because the indirect buffer can be reallocated when
        // the draw count grows.
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("Cull Bind Group"),
            layout: &self.layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: instance_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: self.aabb_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: indirect_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: self.params_buffers[phase].as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 4,
                    resource: wgpu::BindingResource::TextureView(hiz_view),
                },
                wgpu::BindGroupEntry {
                    binding: 5,
                    resource: self.flags_buffer.as_entire_binding(),
                },
            ],
        });

        let mut cpass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
            label: if phase == 0 {
                Some("Instance Cull Phase 1")
            } else {
                Some("Instance Cull Phase 2")
            },
            timestamp_writes: None,
        });
        cpass.set_pipeline(&self.pipeline);
        cpass.set_bind_group(0, &bind_group, &[]);
        cpass.dispatch_workgroups((draw_count as u32).div_ceil(WORKGROUP_SIZE), 1, 1);
    }
}
