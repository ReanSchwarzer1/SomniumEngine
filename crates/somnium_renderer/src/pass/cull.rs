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
    /// Frustum planes + draw count.
    params_buffer: wgpu::Buffer,
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
            params_buffer: device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("Cull Params"),
                size: std::mem::size_of::<GpuCullParams>() as u64,
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            }),
            capacity: INITIAL_CAPACITY,
            staging: Vec::with_capacity(INITIAL_CAPACITY),
        }
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
    pub fn update(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        aabbs: &[GpuCullAabb],
        view_proj: glam::Mat4,
        disabled: bool,
    ) {
        self.staging.clear();
        self.staging.extend_from_slice(aabbs);

        if self.staging.len() > self.capacity {
            let mut cap = self.capacity.max(1);
            while cap < self.staging.len() {
                cap *= 2;
            }
            self.aabb_buffer = Self::alloc_aabbs(device, cap);
            self.capacity = cap;
        }

        if !self.staging.is_empty() {
            queue.write_buffer(&self.aabb_buffer, 0, bytemuck::cast_slice(&self.staging));
        }

        let params = GpuCullParams {
            planes: crate::culling::frustum_planes(view_proj),
            draw_count: self.staging.len() as u32,
            disabled: u32::from(disabled),
            _pad: [0; 2],
        };
        queue.write_buffer(&self.params_buffer, 0, bytemuck::bytes_of(&params));
    }

    /// Dispatch the cull, writing verdicts into `indirect_buffer`.
    pub fn record(
        &self,
        device: &wgpu::Device,
        encoder: &mut wgpu::CommandEncoder,
        instance_buffer: &wgpu::Buffer,
        indirect_buffer: &wgpu::Buffer,
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
                wgpu::BindGroupEntry { binding: 0, resource: instance_buffer.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 1, resource: self.aabb_buffer.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 2, resource: indirect_buffer.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 3, resource: self.params_buffer.as_entire_binding() },
            ],
        });

        let mut cpass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
            label: Some("Instance Cull Pass"),
            timestamp_writes: None,
        });
        cpass.set_pipeline(&self.pipeline);
        cpass.set_bind_group(0, &bind_group, &[]);
        cpass.dispatch_workgroups(
            (draw_count as u32).div_ceil(WORKGROUP_SIZE),
            1,
            1,
        );
    }
}
