//! Material data management for the shading pass.

use bytemuck::{Pod, Zeroable};

/// Material structure that matches the GPU layout in shading.wgsl.
#[repr(C)]
#[derive(Copy, Clone, Debug, Pod, Zeroable)]
pub struct GpuMaterial {
    pub base_color: [f32; 4],
    pub roughness: f32,
    pub metallic: f32,
    pub albedo_map: i32,
    pub normal_map: i32,
    pub metallic_roughness_map: i32,
    pub _padding: [i32; 3],
}

/// Manages a pool of materials in a GPU storage buffer.
pub struct MaterialPool {
    pub buffer: wgpu::Buffer,
    materials: Vec<GpuMaterial>,
}

impl MaterialPool {
    pub fn new(device: &wgpu::Device) -> Self {
        let buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Global Material Buffer"),
            size: 1024 * 64, // 64KB
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        Self {
            buffer,
            materials: Vec::new(),
        }
    }

    /// Add a material to the pool and return its ID.
    pub fn add_material(&mut self, queue: &wgpu::Queue, material: GpuMaterial) -> u32 {
        let id = self.materials.len() as u32;
        self.materials.push(material);
        
        // Update the buffer
        queue.write_buffer(&self.buffer, (id as usize * std::mem::size_of::<GpuMaterial>()) as u64, bytemuck::bytes_of(&material));
        
        id
    }
}
