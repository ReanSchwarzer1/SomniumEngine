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

    /// Local-space bounding box per mesh, keyed by `vertex_offset` (Phase 15B).
    ///
    /// GPU frustum culling needs each draw's bounds. Keying on the allocation's
    /// vertex offset means existing `DrawCommand` call sites need no changes —
    /// the renderer looks the box up when it builds the cull buffer, and a draw
    /// with no entry is simply never culled.
    aabbs: std::collections::HashMap<u32, ([f32; 3], [f32; 3])>,
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
            aabbs: std::collections::HashMap::new(),
        }
    }

    /// Upload mesh data to the GPU and return its allocation info.
    pub fn upload_mesh(&mut self, queue: &wgpu::Queue, vertices: &[Vertex], indices: &[u32], material_id: u32) -> MeshAllocation {
        debug_assert_indices_in_range(vertices.len(), indices);
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
        debug_assert_indices_in_range(vertices.len(), indices);
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

    /// Local-space bounds of the mesh at `vertex_offset`, if it is known.
    pub fn mesh_aabb(&self, vertex_offset: u32) -> Option<([f32; 3], [f32; 3])> {
        self.aabbs.get(&vertex_offset).copied()
    }

    fn write_mesh(&mut self, queue: &wgpu::Queue, alloc: &MeshAllocation, vertices: &[Vertex], indices: &[u32]) {
        // Record local bounds for GPU culling (Phase 15B). Both upload paths
        // funnel through here, so every mesh gets one.
        self.aabbs.insert(alloc.vertex_offset, compute_aabb(vertices));

        // Phase 15C: the visibility buffer packs the primitive index into 16
        // bits, so a larger mesh would wrap and shade the wrong triangle.
        // Warn rather than fail — the mesh still renders, just not past the
        // limit — so this is diagnosable instead of a silent corruption.
        let triangles = indices.len() as u32 / 3;
        if triangles > crate::command::MAX_TRIANGLES_PER_DRAW {
            tracing::warn!(
                triangles,
                limit = crate::command::MAX_TRIANGLES_PER_DRAW,
                "mesh exceeds the visibility buffer's per-draw triangle limit;                  primitive IDs past the limit will wrap. Split the mesh."
            );
        }

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

/// Local-space AABB of a vertex list.
///
/// An empty mesh yields an inverted box (min > max), which the culling test
/// treats as "never visible" — correct, since there is nothing to draw.
fn compute_aabb(vertices: &[Vertex]) -> ([f32; 3], [f32; 3]) {
    let mut min = [f32::INFINITY; 3];
    let mut max = [f32::NEG_INFINITY; 3];
    for v in vertices {
        for axis in 0..3 {
            min[axis] = min[axis].min(v.position[axis]);
            max[axis] = max[axis].max(v.position[axis]);
        }
    }
    (min, max)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn vert(p: [f32; 3]) -> Vertex {
        Vertex { position: p, normal: [0.0, 1.0, 0.0], uv: [0.0, 0.0] }
    }

    #[test]
    fn aabb_wraps_every_vertex() {
        let verts = [
            vert([-1.0, 0.0, 2.0]),
            vert([3.0, -4.0, 0.5]),
            vert([0.0, 1.0, -6.0]),
        ];
        let (min, max) = compute_aabb(&verts);
        assert_eq!(min, [-1.0, -4.0, -6.0]);
        assert_eq!(max, [3.0, 1.0, 2.0]);
    }

    #[test]
    fn empty_mesh_yields_an_inverted_box() {
        let (min, max) = compute_aabb(&[]);
        assert!(min[0] > max[0], "empty mesh must not produce a visible box");
    }
}

/// Every index must address a vertex this mesh actually uploaded.
///
/// The pool is a bump allocator, so an index past `vertex_count` silently reads
/// into whichever mesh was uploaded next: the shader pulls a valid-looking
/// vertex from unrelated geometry and stretches a triangle to it, which on
/// screen is a fan of long thin shards radiating out of the model.
/// Nothing traps this on the GPU, so it is worth one pass over the indices at
/// upload time.
fn debug_assert_indices_in_range(vertex_count: usize, indices: &[u32]) {
    if let Some(&max) = indices.iter().max() {
        if max as usize >= vertex_count {
            tracing::warn!(
                "mesh upload: index {max} exceeds its {vertex_count} vertices                  ({} indices) — geometry will render as stray triangles",
                indices.len(),
            );
        }
    }
}
