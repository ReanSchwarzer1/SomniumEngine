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

/// Packed unsigned triangle SDF (Phase 24P). 16³, keyed with the mesh AABB.
#[derive(Clone, Debug)]
pub struct MeshSdfBrick {
    pub min: [f32; 3],
    pub max: [f32; 3],
    pub dist: Vec<f32>,
}

/// Side length of a packed mesh-SDF brick.
pub const MESH_SDF_BRICK: u32 = 16;

/// Cap on triangles walked when baking a brick. Photoscanned trees are huge;
/// the clipmap is 64³ so extra triangles past this do not change the look.
const MESH_SDF_TRI_CAP: usize = 1024;

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

    /// Packed unsigned triangle SDF per static mesh (Phase 24P).
    sdf_bricks: std::collections::HashMap<u32, std::sync::Arc<MeshSdfBrick>>,

    /// Reserved vertex spans, `offset -> capacity` (Phase 25A-2).
    ///
    /// Only spans handed out by [`GeometryPool::reserve_vertices`] appear here.
    /// They exist so a rewrite that outgrew its reservation is refused rather
    /// than walking into the next span — which for terrain would mean one
    /// chunk's heights overwriting its neighbour's, and nothing on the GPU
    /// traps that.
    vertex_spans: std::collections::HashMap<u32, u32>,
    index_spans: std::collections::HashMap<u32, u32>,

    /// Actual pool sizes after clamping to the device limit.
    vertex_bytes: u64,
    index_bytes: u64,
}

fn geometry_usage(base: wgpu::BufferUsages, ray_query_enabled: bool) -> wgpu::BufferUsages {
    if ray_query_enabled {
        base | wgpu::BufferUsages::BLAS_INPUT
    } else {
        base
    }
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

        let ray_query_enabled = device
            .features()
            .contains(crate::context::RAY_TRACING_FEATURES);
        let vertex_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Global Vertex Buffer"),
            size: vertex_bytes,
            // Phase 24J: BLAS_INPUT lets the acceleration-structure build read
            // positions straight out of the shared pool, so ray tracing needs
            // no second copy of the geometry.
            usage: geometry_usage(
                wgpu::BufferUsages::STORAGE
                    | wgpu::BufferUsages::COPY_DST
                    | wgpu::BufferUsages::VERTEX,
                ray_query_enabled,
            ),
            mapped_at_creation: false,
        });

        let index_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Global Index Buffer"),
            size: index_bytes,
            usage: geometry_usage(
                wgpu::BufferUsages::STORAGE
                    | wgpu::BufferUsages::COPY_DST
                    | wgpu::BufferUsages::INDEX,
                ray_query_enabled,
            ),
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
            sdf_bricks: std::collections::HashMap::new(),
            vertex_spans: std::collections::HashMap::new(),
            index_spans: std::collections::HashMap::new(),
            vertex_bytes,
            index_bytes,
        }
    }

    /// Upload mesh data to the GPU and return its allocation info.
    pub fn upload_mesh(
        &mut self,
        queue: &wgpu::Queue,
        vertices: &[Vertex],
        indices: &[u32],
        material_id: u32,
    ) -> MeshAllocation {
        debug_assert_indices_in_range(vertices.len(), indices);
        if let Some(empty) = self.reject_if_full(vertices.len(), indices.len(), material_id) {
            return empty;
        }

        // Phase 15D: cluster the mesh and upload the permuted index buffer, so
        // each meshlet is a contiguous range that 15F can draw directly.
        // Triangle order within a draw does not affect the image.
        let build = crate::meshlet::build_meshlets(vertices, indices);
        let indices: &[u32] = if build.meshlets.is_empty() {
            indices
        } else {
            &build.indices
        };

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
        if let Some(brick) = bake_mesh_sdf(vertices, indices) {
            self.sdf_bricks.insert(v_offset, std::sync::Arc::new(brick));
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
    pub fn upload_mesh_pooled(
        &mut self,
        queue: &wgpu::Queue,
        vertices: &[Vertex],
        indices: &[u32],
        material_id: u32,
    ) -> MeshAllocation {
        debug_assert_indices_in_range(vertices.len(), indices);
        let v_count = vertices.len() as u32;
        let i_count = indices.len() as u32;

        let reuse = self
            .free_blocks
            .iter()
            .position(|b| b.vertex_capacity >= v_count && b.index_capacity >= i_count);

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

    // ── Rewritable spans (Phase 25A-2) ──────────────────────────────────────
    //
    // Terrain chunks need to live in the global pool so `shading.wgsl` can pull
    // their attributes with the code path it already has. The question that
    // opened is what happens when a sculpt stroke changes a chunk: `upload_mesh`
    // is a load-time bump allocator, and the voxel free list (`upload_mesh_pooled`
    // / `free_mesh`) hands back a *different* offset every time.
    //
    // Neither is what terrain wants, because of one fact about the data: a
    // chunk's vertex count is fixed by the descriptor at `(chunk_cells + 1)²`
    // and never changes. Sculpting rewrites height *values*, not counts, and a
    // coarser LOD skips vertices through the index buffer rather than rebuilding
    // the grid. So a chunk can be reserved exactly once and rewritten in place
    // for the rest of its life.
    //
    // That matters beyond tidiness: `vertex_offset` is the key for the AABB map,
    // the meshlet map, and (in 25B) the per-mesh BLAS. Free-list churn would
    // invalidate all three on every brush dab and leave the old entries behind,
    // since `free_mesh` does not remove them. A stable span keeps every one of
    // those lookups correct for free.
    //
    // Indices go the other way. Index data depends only on `(lod, edge_mask)`
    // and is *chunk-relative* — the visibility pass reads
    // `vertices[vertex_offset + indices[index_offset + i]]` — so one span per
    // key is shared by every chunk, allocated on first use and never rewritten.
    // There are at most 5 LODs × 16 masks, about 2 MB of the index pool if
    // every combination is ever needed.
    //
    // Neither span is ever released today: a terrain lives as long as the
    // renderer does. When terrain deletion arrives it wants a `release_*` pair
    // feeding the free list — and because every chunk span is the same size,
    // first-fit reuse would be exact, with no fragmentation.

    /// Reserve a vertex span to be rewritten in place, or `None` if the pool
    /// cannot fit it.
    ///
    /// The caller keeps the returned offset for the lifetime of the geometry and
    /// passes it back to [`GeometryPool::write_vertices`] on every rebuild.
    pub fn reserve_vertices(&mut self, count: u32) -> Option<u32> {
        let offset = reserve_span(
            &mut self.next_vertex,
            count,
            std::mem::size_of::<Vertex>() as u64,
            self.vertex_bytes,
        );
        match offset {
            Some(offset) => {
                self.vertex_spans.insert(offset, count);
                Some(offset)
            }
            None => {
                tracing::error!(
                    "geometry pool full: cannot reserve {count} vertices; pool holds {:.0} MB",
                    self.vertex_bytes as f64 / 1048576.0,
                );
                None
            }
        }
    }

    /// Reserve an index span. See [`GeometryPool::reserve_vertices`].
    pub fn reserve_indices(&mut self, count: u32) -> Option<u32> {
        match reserve_span(&mut self.next_index, count, 4, self.index_bytes) {
            Some(offset) => {
                self.index_spans.insert(offset, count);
                Some(offset)
            }
            None => {
                tracing::error!(
                    "geometry pool full: cannot reserve {count} indices; pool holds {:.0} MB",
                    self.index_bytes as f64 / 1048576.0,
                );
                None
            }
        }
    }

    /// Rewrite a reserved vertex span and refresh its recorded bounds.
    ///
    /// The AABB has to be refreshed here, not only at reservation: sculpting
    /// changes a chunk's height range, and a stale box is what makes GPU
    /// culling drop geometry that is plainly on screen.
    pub fn write_vertices(&mut self, queue: &wgpu::Queue, offset: u32, vertices: &[Vertex]) {
        if !span_accepts(&self.vertex_spans, offset, vertices.len(), "vertices") {
            return;
        }
        self.aabbs.insert(offset, compute_aabb(vertices));
        queue.write_buffer(
            &self.vertex_buffer,
            offset as u64 * std::mem::size_of::<Vertex>() as u64,
            bytemuck::cast_slice(vertices),
        );
    }

    /// Write a reserved index span.
    pub fn write_indices(&self, queue: &wgpu::Queue, offset: u32, indices: &[u32]) {
        if !span_accepts(&self.index_spans, offset, indices.len(), "indices") {
            return;
        }
        queue.write_buffer(
            &self.index_buffer,
            offset as u64 * std::mem::size_of::<u32>() as u64,
            bytemuck::cast_slice(indices),
        );
    }

    /// Return a pooled allocation's block to the free list.
    pub fn free_mesh(&mut self, alloc: MeshAllocation) {
        if alloc.vertex_capacity == 0 && alloc.index_capacity == 0 {
            return;
        }
        self.sdf_bricks.remove(&alloc.vertex_offset);
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
        let v_end =
            (self.next_vertex as u64 + vertex_count as u64) * std::mem::size_of::<Vertex>() as u64;
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

    /// Packed triangle SDF for the mesh at `vertex_offset`, if one was baked.
    pub fn mesh_sdf(&self, vertex_offset: u32) -> Option<std::sync::Arc<MeshSdfBrick>> {
        self.sdf_bricks.get(&vertex_offset).cloned()
    }

    fn write_mesh(
        &mut self,
        queue: &wgpu::Queue,
        alloc: &MeshAllocation,
        vertices: &[Vertex],
        indices: &[u32],
    ) {
        // Record local bounds for GPU culling (Phase 15B). Both upload paths
        // funnel through here, so every mesh gets one.
        self.aabbs
            .insert(alloc.vertex_offset, compute_aabb(vertices));

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

/// Bump-allocate `count` elements of `stride` bytes, or `None` if the pool
/// cannot hold them.
///
/// Split out from the two `reserve_*` methods so the arithmetic can be tested
/// without a GPU device — the failure it guards against (moving the bump
/// pointer past the end and letting every later write land somewhere wrong) is
/// silent on the GPU. The `u64` maths is deliberate: the multiply overflows
/// `u32` well inside a 256 MB pool.
fn reserve_span(next: &mut u32, count: u32, stride: u64, capacity_bytes: u64) -> Option<u32> {
    let end = (*next as u64 + count as u64) * stride;
    if end > capacity_bytes {
        return None;
    }
    let offset = *next;
    *next += count;
    Some(offset)
}

/// Whether a write of `len` elements fits the span reserved at `offset`.
fn span_accepts(
    spans: &std::collections::HashMap<u32, u32>,
    offset: u32,
    len: usize,
    what: &str,
) -> bool {
    let Some(&capacity) = spans.get(&offset) else {
        tracing::error!("write of {len} {what} at unreserved offset {offset}; skipped");
        return false;
    };
    if len as u64 > capacity as u64 {
        tracing::error!(
            "write of {len} {what} exceeds the {capacity} reserved at offset {offset}; skipped",
        );
        return false;
    }
    true
}

fn bake_mesh_sdf(vertices: &[Vertex], indices: &[u32]) -> Option<MeshSdfBrick> {
    if vertices.is_empty() || indices.len() < 3 {
        return None;
    }
    let (min, max) = compute_aabb(vertices);
    if !min[0].is_finite() || min[0] > max[0] {
        return None;
    }
    let min_v = glam::Vec3::from_array(min);
    let max_v = glam::Vec3::from_array(max);
    let extent = (max_v - min_v).max(glam::Vec3::splat(1e-3));
    let pad = extent.max_element() * 0.08 + 0.02;
    let bmin = min_v - glam::Vec3::splat(pad);
    let bmax = max_v + glam::Vec3::splat(pad);
    let bext = (bmax - bmin).max(glam::Vec3::splat(1e-4));
    let n = MESH_SDF_BRICK;
    let mut dist = vec![f32::MAX; (n * n * n) as usize];
    let tri_count = (indices.len() / 3).min(MESH_SDF_TRI_CAP);
    for z in 0..n {
        for y in 0..n {
            for x in 0..n {
                let uvw = (glam::Vec3::new(x as f32, y as f32, z as f32) + glam::Vec3::splat(0.5))
                    / n as f32;
                let p = bmin + uvw * bext;
                let mut d = f32::MAX;
                for t in 0..tri_count {
                    let ia = indices[t * 3] as usize;
                    let ib = indices[t * 3 + 1] as usize;
                    let ic = indices[t * 3 + 2] as usize;
                    if ia >= vertices.len() || ib >= vertices.len() || ic >= vertices.len() {
                        continue;
                    }
                    let a = glam::Vec3::from_array(vertices[ia].position);
                    let b = glam::Vec3::from_array(vertices[ib].position);
                    let c = glam::Vec3::from_array(vertices[ic].position);
                    d = d.min(point_triangle_distance(p, a, b, c));
                }
                dist[(z * n * n + y * n + x) as usize] = d;
            }
        }
    }
    Some(MeshSdfBrick {
        min: bmin.to_array(),
        max: bmax.to_array(),
        dist,
    })
}

/// Unsigned distance from `p` to triangle `abc` (Ericson).
fn point_triangle_distance(p: glam::Vec3, a: glam::Vec3, b: glam::Vec3, c: glam::Vec3) -> f32 {
    (closest_point_on_triangle(p, a, b, c) - p).length()
}

fn closest_point_on_triangle(
    p: glam::Vec3,
    a: glam::Vec3,
    b: glam::Vec3,
    c: glam::Vec3,
) -> glam::Vec3 {
    let ab = b - a;
    let ac = c - a;
    let ap = p - a;
    let d1 = ab.dot(ap);
    let d2 = ac.dot(ap);
    if d1 <= 0.0 && d2 <= 0.0 {
        return a;
    }
    let bp = p - b;
    let d3 = ab.dot(bp);
    let d4 = ac.dot(bp);
    if d3 >= 0.0 && d4 <= d3 {
        return b;
    }
    let vc = d1 * d4 - d3 * d2;
    if vc <= 0.0 && d1 >= 0.0 && d3 <= 0.0 {
        let v = d1 / (d1 - d3);
        return a + ab * v;
    }
    let cp = p - c;
    let d5 = ab.dot(cp);
    let d6 = ac.dot(cp);
    if d6 >= 0.0 && d5 <= d6 {
        return c;
    }
    let vb = d5 * d2 - d1 * d6;
    if vb <= 0.0 && d2 >= 0.0 && d6 <= 0.0 {
        let w = d2 / (d2 - d6);
        return a + ac * w;
    }
    let va = d3 * d6 - d5 * d4;
    if va <= 0.0 && (d4 - d3) >= 0.0 && (d5 - d6) >= 0.0 {
        let w = (d4 - d3) / ((d4 - d3) + (d5 - d6));
        return b + (c - b) * w;
    }
    let denom = va + vb + vc;
    if denom.abs() < 1e-12 {
        return a;
    }
    let v = vb / denom;
    let w = vc / denom;
    a + ab * v + ac * w
}

/// Sample a packed brick with trilinear interpolation. `None` if `local` is
/// outside the brick.
pub fn sample_mesh_sdf(brick: &MeshSdfBrick, local: glam::Vec3) -> Option<f32> {
    let min = glam::Vec3::from_array(brick.min);
    let max = glam::Vec3::from_array(brick.max);
    let extent = (max - min).max(glam::Vec3::splat(1e-4));
    let uvw = (local - min) / extent;
    if uvw.x < 0.0 || uvw.y < 0.0 || uvw.z < 0.0 || uvw.x > 1.0 || uvw.y > 1.0 || uvw.z > 1.0 {
        return None;
    }
    let n = MESH_SDF_BRICK as f32;
    let p =
        (uvw * n - glam::Vec3::splat(0.5)).clamp(glam::Vec3::ZERO, glam::Vec3::splat(n - 1.001));
    let i0 = p.floor().as_ivec3();
    let f = p.fract();
    let n_i = MESH_SDF_BRICK as i32;
    let at = |x: i32, y: i32, z: i32| {
        let x = x.clamp(0, n_i - 1) as u32;
        let y = y.clamp(0, n_i - 1) as u32;
        let z = z.clamp(0, n_i - 1) as u32;
        brick.dist[(z * MESH_SDF_BRICK * MESH_SDF_BRICK + y * MESH_SDF_BRICK + x) as usize]
    };
    let d000 = at(i0.x, i0.y, i0.z);
    let d100 = at(i0.x + 1, i0.y, i0.z);
    let d010 = at(i0.x, i0.y + 1, i0.z);
    let d110 = at(i0.x + 1, i0.y + 1, i0.z);
    let d001 = at(i0.x, i0.y, i0.z + 1);
    let d101 = at(i0.x + 1, i0.y, i0.z + 1);
    let d011 = at(i0.x, i0.y + 1, i0.z + 1);
    let d111 = at(i0.x + 1, i0.y + 1, i0.z + 1);
    let x00 = d000 + (d100 - d000) * f.x;
    let x10 = d010 + (d110 - d010) * f.x;
    let x01 = d001 + (d101 - d001) * f.x;
    let x11 = d011 + (d111 - d011) * f.x;
    let y0 = x00 + (x10 - x00) * f.y;
    let y1 = x01 + (x11 - x01) * f.y;
    Some(y0 + (y1 - y0) * f.z)
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
        Vertex {
            position: p,
            normal: [0.0, 1.0, 0.0],
            uv: [0.0, 0.0],
        }
    }

    #[test]
    fn blas_usage_is_present_only_when_ray_queries_are_enabled() {
        let base = wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::VERTEX;
        assert_eq!(geometry_usage(base, false), base);
        assert!(geometry_usage(base, true).contains(wgpu::BufferUsages::BLAS_INPUT));
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

    #[test]
    fn a_unit_triangle_sdf_is_near_zero_on_the_surface() {
        let verts = [
            vert([0.0, 0.0, 0.0]),
            vert([1.0, 0.0, 0.0]),
            vert([0.0, 1.0, 0.0]),
        ];
        let brick = bake_mesh_sdf(&verts, &[0, 1, 2]).expect("bake");
        let on = sample_mesh_sdf(&brick, glam::Vec3::new(0.25, 0.25, 0.0)).unwrap();
        let away = sample_mesh_sdf(&brick, glam::Vec3::new(0.25, 0.25, 0.05)).unwrap();
        assert!(on < 0.12, "surface sample {on}");
        assert!(
            away > on,
            "distance must grow off the plane ({away} vs {on})"
        );
        assert_eq!(brick.dist.len(), (MESH_SDF_BRICK.pow(3)) as usize);
    }

    // ── Rewritable spans (Phase 25A-2) ──────────────────────────────────────

    #[test]
    fn spans_are_handed_out_back_to_back() {
        let mut next = 0u32;
        assert_eq!(reserve_span(&mut next, 100, 32, 1 << 20), Some(0));
        assert_eq!(reserve_span(&mut next, 40, 32, 1 << 20), Some(100));
        assert_eq!(next, 140);
    }

    #[test]
    fn a_reservation_past_the_end_is_refused_without_moving_the_pointer() {
        // The whole reason this is a separate function: the bump pointer moving
        // on a rejected reservation is what corrupts every later mesh, and
        // nothing on the GPU reports it.
        let mut next = 10u32;
        assert_eq!(reserve_span(&mut next, 1000, 32, 320), None);
        assert_eq!(next, 10, "a refused reservation must not consume space");
    }

    #[test]
    fn a_span_ending_exactly_at_the_pool_end_still_fits() {
        let mut next = 0u32;
        assert_eq!(reserve_span(&mut next, 10, 32, 320), Some(0));
        assert_eq!(reserve_span(&mut next, 1, 32, 320), None);
    }

    #[test]
    fn span_arithmetic_is_done_in_u64() {
        // A 256 MB vertex pool holds ~8.4 M vertices, and the byte figure for a
        // request that large overflows u32 if the multiply happens before the
        // widening — 200 M vertices is 6.4 GB, which wraps to 2.1 GB and would
        // read as "fits" in 32-bit maths.
        let mut next = 0u32;
        assert!(reserve_span(&mut next, 200_000_000, 32, 1024 * 1024 * 256).is_none());
        assert_eq!(next, 0);

        // And the ordinary case just past a full pool.
        let mut next = 8_000_000u32;
        assert!(reserve_span(&mut next, 1_000_000, 32, 1024 * 1024 * 256).is_none());
        assert_eq!(next, 8_000_000);
    }

    #[test]
    fn a_rewrite_may_be_shorter_than_its_reservation_but_never_longer() {
        // A terrain chunk always rewrites exactly its reserved count, but the
        // guard is what stops one chunk's heights landing in its neighbour's
        // span if that ever stops being true.
        let mut spans = std::collections::HashMap::new();
        spans.insert(64u32, 100u32);
        assert!(span_accepts(&spans, 64, 100, "vertices"));
        assert!(span_accepts(&spans, 64, 1, "vertices"));
        assert!(!span_accepts(&spans, 64, 101, "vertices"));
        assert!(
            !span_accepts(&spans, 65, 1, "vertices"),
            "offset must be a reserved span"
        );
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
