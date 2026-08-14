//! Instance data management for the Visibility Buffer pipeline.

use bytemuck::{Pod, Zeroable};

/// Per-instance data matching the GPU layout.
#[repr(C)]
#[derive(Copy, Clone, Debug, Pod, Zeroable)]
pub struct GpuInstanceData {
    pub model_matrix: [[f32; 4]; 4],
    pub material_id: u32,
    pub mesh_vertex_offset: u32,
    pub mesh_index_offset: u32,
    pub _padding: u32,
}

/// Manages per-instance data in a GPU storage buffer.
pub struct InstancePool {
    pub buffer: wgpu::Buffer,
    instances: Vec<GpuInstanceData>,
}

impl InstancePool {
    pub fn new(device: &wgpu::Device) -> Self {
        let buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Global Instance Buffer"),
            size: 1024 * 1024 * 16, // 16MB
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        Self {
            buffer,
            instances: Vec::with_capacity(256),
        }
    }

    /// Clear instances for the current frame.
    pub fn clear(&mut self) {
        self.instances.clear();
    }

    /// Add an instance to the current frame's batch.
    pub fn add_instance(&mut self, data: GpuInstanceData) -> u32 {
        let id = self.instances.len() as u32;
        self.instances.push(data);
        id
    }

    /// Upload all instances to the GPU.
    pub fn upload(&self, queue: &wgpu::Queue) {
        if !self.instances.is_empty() {
            queue.write_buffer(&self.buffer, 0, bytemuck::cast_slice(&self.instances));
        }
    }
}
