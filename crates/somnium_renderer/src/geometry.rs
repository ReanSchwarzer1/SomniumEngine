//! Geometry management for the Visibility Buffer pipeline.

use somnium_asset::Vertex;

/// Information about an allocated mesh in the global buffers.
///
/// `*_capacity` is the size of the underlying block, which can exceed the
/// live counts when a pooled allocation reuses a larger freed block
/// (Phase 14). For plain bump allocations capacity == count.
#[derive(Debug, Clone, Copy)]
pub struct MeshAllocation {
    pub vertex_offset: u32,
    pub vertex_count: u32,
    pub index_offset: u32,
    pub index_count: u32,
    pub material_id: u32,
    pub vertex_capacity: u32,
    pub index_capacity: u32,
}

/// A freed region of the global buffers, reusable by pooled uploads.
#[derive(Debug, Clone, Copy)]
struct FreeBlock {
    vertex_offset: u32,
    vertex_capacity: u32,
    index_offset: u32,
    index_capacity: u32,
}

/// Manages large GPU buffers for all scene geometry.
pub struct GeometryPool {
    pub vertex_buffer: wgpu::Buffer,
    pub index_buffer: wgpu::Buffer,

    // Bump allocator for static meshes (Phase 7), plus a first-fit free list
    // for dynamic meshes that are freed and reallocated (Phase 14 voxel chunks).
    next_vertex: u32,
    next_index: u32,
    free_blocks: Vec<FreeBlock>,
}

impl GeometryPool {
    pub fn new(device: &wgpu::Device) -> Self {
        let vertex_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Global Vertex Buffer"),
            size: 1024 * 1024 * 64, // 64MB
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::VERTEX,
            mapped_at_creation: false,
        });

        let index_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Global Index Buffer"),
            size: 1024 * 1024 * 32, // 32MB
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::INDEX,
            mapped_at_creation: false,
        });

        Self {
            vertex_buffer,
            index_buffer,
            next_vertex: 0,
            next_index: 0,
            free_blocks: Vec::new(),
        }
    }

    /// Upload mesh data to the GPU and return its allocation info.
    pub fn upload_mesh(&mut self, queue: &wgpu::Queue, vertices: &[Vertex], indices: &[u32], material_id: u32) -> MeshAllocation {
        let v_offset = self.next_vertex;
        let i_offset = self.next_index;

        self.next_vertex += vertices.len() as u32;
        self.next_index += indices.len() as u32;

        let alloc = MeshAllocation {
            vertex_offset: v_offset,
            vertex_count: vertices.len() as u32,
            index_offset: i_offset,
            index_count: indices.len() as u32,
            material_id,
            vertex_capacity: vertices.len() as u32,
            index_capacity: indices.len() as u32,
        };
        self.write_mesh(queue, &alloc, vertices, indices);
        alloc
    }

    /// Upload a dynamic mesh, reusing a freed block when one is big enough
    /// (first-fit). Pair with [`GeometryPool::free_mesh`] — used by the Phase
    /// 14 voxel chunks, which are remeshed and despawned continuously.
    pub fn upload_mesh_pooled(&mut self, queue: &wgpu::Queue, vertices: &[Vertex], indices: &[u32], material_id: u32) -> MeshAllocation {
        let v_count = vertices.len() as u32;
        let i_count = indices.len() as u32;

        let reuse = self.free_blocks.iter().position(|b| {
            b.vertex_capacity >= v_count && b.index_capacity >= i_count
        });

        let alloc = if let Some(slot) = reuse {
            let block = self.free_blocks.swap_remove(slot);
            MeshAllocation {
                vertex_offset: block.vertex_offset,
                vertex_count: v_count,
                index_offset: block.index_offset,
                index_count: i_count,
                material_id,
                vertex_capacity: block.vertex_capacity,
                index_capacity: block.index_capacity,
            }
        } else {
            let alloc = MeshAllocation {
                vertex_offset: self.next_vertex,
                vertex_count: v_count,
                index_offset: self.next_index,
                index_count: i_count,
                material_id,
                vertex_capacity: v_count,
                index_capacity: i_count,
            };
            self.next_vertex += v_count;
            self.next_index += i_count;
            alloc
        };

        self.write_mesh(queue, &alloc, vertices, indices);
        alloc
    }

    /// Return a pooled allocation's block to the free list.
    pub fn free_mesh(&mut self, alloc: MeshAllocation) {
        if alloc.vertex_capacity == 0 && alloc.index_capacity == 0 {
            return;
        }
        self.free_blocks.push(FreeBlock {
            vertex_offset: alloc.vertex_offset,
            vertex_capacity: alloc.vertex_capacity,
            index_offset: alloc.index_offset,
            index_capacity: alloc.index_capacity,
        });
    }

    fn write_mesh(&self, queue: &wgpu::Queue, alloc: &MeshAllocation, vertices: &[Vertex], indices: &[u32]) {
        queue.write_buffer(
            &self.vertex_buffer,
            alloc.vertex_offset as u64 * std::mem::size_of::<Vertex>() as u64,
            bytemuck::cast_slice(vertices),
        );
        queue.write_buffer(
            &self.index_buffer,
            alloc.index_offset as u64 * std::mem::size_of::<u32>() as u64,
            bytemuck::cast_slice(indices),
        );
    }
}
