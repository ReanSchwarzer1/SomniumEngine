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

/// Vertex pool size we ask for.
///
/// Raised from 64 MB in Phase 17E: a single photoscanned tree runs to millions
/// of triangles, and the previous budget could not hold one alongside the rest
/// of a scene. 256 MB covers roughly 8 million vertices.
///
/// Clamped at construction to the device's `max_storage_buffer_binding_size` —
/// these are bound as storage buffers for programmable vertex pulling, and
/// wgpu's default ceiling is 128 MB. Binding one larger than the limit is a
/// validation error at bind-group creation, not at buffer creation, so it
/// surfaces as a crash on the first frame rather than a clean failure.
const VERTEX_POOL_BYTES: u64 = 1024 * 1024 * 256;

/// Index pool size we ask for — 128 MB, about 32 million indices.
const INDEX_POOL_BYTES: u64 = 1024 * 1024 * 128;

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

    /// Phase 15D: per-mesh clusters, keyed by `vertex_offset` like `aabbs`.
    ///
    /// Only static uploads are clustered. Pooled uploads are the voxel chunks,
    /// which are remeshed continuously as the camera moves — paying the sort on
    /// every remesh would cost more than the culling saves, and a chunk is
    /// already small enough to cull as a unit.
    meshlets: std::collections::HashMap<u32, Vec<crate::meshlet::Meshlet>>,

    /// Actual pool sizes after clamping to the device limit.
    vertex_bytes: u64,
    index_bytes: u64,
}

impl GeometryPool {
    pub fn new(device: &wgpu::Device) -> Self {
        let max_binding = u64::from(device.limits().max_storage_buffer_binding_size);
        let vertex_bytes = VERTEX_POOL_BYTES.min(max_binding);
        let index_bytes = INDEX_POOL_BYTES.min(max_binding);
        tracing::info!(
            "Geometry pool: {:.0} MB vertex / {:.0} MB index (device limit {:.0} MB)",
            vertex_bytes as f64 / 1048576.0,
            index_bytes as f64 / 1048576.0,
            max_binding as f64 / 1048576.0,
        );

        let vertex_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Global Vertex Buffer"),
            size: vertex_bytes,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::VERTEX,
            mapped_at_creation: false,
        });

        let index_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Global Index Buffer"),
            size: index_bytes,
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
            meshlets: std::collections::HashMap::new(),
            vertex_bytes,
            index_bytes,
        }
    }

    /// Upload mesh data to the GPU and return its allocation info.
    pub fn upload_mesh(&mut self, queue: &wgpu::Queue, vertices: &[Vertex], indices: &[u32], material_id: u32) -> MeshAllocation {
        debug_assert_indices_in_range(vertices.len(), indices);
        if let Some(empty) = self.reject_if_full(vertices.len(), indices.len(), material_id) {
            return empty;
        }

        // Phase 15D: cluster the mesh and upload the permuted index buffer, so
        // each meshlet is a contiguous range that 15F can draw directly.
        // Triangle order within a draw does not affect the image.
        let build = crate::meshlet::build_meshlets(vertices, indices);
        let indices: &[u32] = if build.meshlets.is_empty() { indices } else { &build.indices };

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
        if !build.meshlets.is_empty() {
            self.meshlets.insert(v_offset, build.meshlets);
        }
        alloc
    }

    /// Clusters for the mesh at `vertex_offset`, if it was clustered.
    pub fn mesh_meshlets(&self, vertex_offset: u32) -> Option<&[crate::meshlet::Meshlet]> {
        self.meshlets.get(&vertex_offset).map(|v| v.as_slice())
    }

    /// Total clusters across every uploaded mesh. Diagnostic for 15E/15F.
    pub fn meshlet_count(&self) -> usize {
        self.meshlets.values().map(|m| m.len()).sum()
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

    /// Refuse an upload that would not fit, returning an empty allocation.
    ///
    /// The pool is a bump allocator writing at `offset * stride`, so an
    /// oversized mesh would run off the end of the buffer. wgpu rejects the
    /// write, but the allocation counters have already moved — every later mesh
    /// then reads from the wrong place, which shows up as geometry stretched
    /// between unrelated parts of the scene rather than as an error.
    ///
    /// An empty allocation draws nothing, so one mesh goes missing instead of
    /// the rest of the scene being corrupted.
    fn reject_if_full(
        &self,
        vertex_count: usize,
        index_count: usize,
        material_id: u32,
    ) -> Option<MeshAllocation> {
        let v_end = (self.next_vertex as u64 + vertex_count as u64)
            * std::mem::size_of::<Vertex>() as u64;
        let i_end = (self.next_index as u64 + index_count as u64) * 4;
        if v_end <= self.vertex_bytes && i_end <= self.index_bytes {
            return None;
        }
        tracing::error!(
            "geometry pool full: mesh of {vertex_count} vertices / {index_count} indices              needs {:.1} MB vertex and {:.1} MB index, pool holds {:.0}/{:.0} MB.              Mesh skipped.",
            v_end as f64 / 1048576.0,
            i_end as f64 / 1048576.0,
            self.vertex_bytes as f64 / 1048576.0,
            self.index_bytes as f64 / 1048576.0,
        );
        Some(MeshAllocation {
            vertex_offset: 0,
            vertex_count: 0,
            index_offset: 0,
            index_count: 0,
            material_id,
            vertex_capacity: 0,
            index_capacity: 0,
        })
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
