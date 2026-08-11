//! Phase 15A: GPU-driven indirect draw arguments.
//!
//! Instead of the CPU issuing one `draw()` per object, every draw's parameters
//! live in a GPU buffer and the whole scene is submitted with a single
//! `multi_draw_indirect` call. That alone saves CPU time, but the real point is
//! Phase 15B: once the arguments live on the GPU, a compute shader can cull
//! instances by zeroing their `instance_count` without any CPU round-trip.
//!
//! ## Reference Architecture
//!
//! `example_repo/UnrealEngine-release/.../Engine/Shaders/Shared/InstanceCullingDefinitions.h`
//! — UE5's instance-culling pass writes indirect args from a compute shader and
//! flags draws it has culled. Somnium follows the same shape: a dense arg array
//! parallel to the instance buffer, with `instance_count` acting as the
//! keep/discard flag.
//!
//! The visibility pass uses programmable vertex pulling (no vertex/index buffer
//! is bound), so these are **non-indexed** draw args: `vertex_count` is the
//! mesh's index count and the shader pulls indices from the geometry pool.
//! `first_instance` is the draw's slot in the instance buffer, which is why
//! `INDIRECT_FIRST_INSTANCE` is required.

use crate::command::DrawCommand;
use crate::culling::GpuCullAabb;
use crate::meshlet::Meshlet;

/// One non-indexed indirect draw, matching `wgpu`'s expected layout exactly.
///
/// **Size**: 16 bytes. Field order and packing are dictated by the GPU — do not
/// reorder.
#[repr(C)]
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq, bytemuck::Pod, bytemuck::Zeroable)]
pub struct DrawIndirectArgs {
    /// Vertices to draw — here, the mesh's index count.
    pub vertex_count: u32,
    /// Instances to draw. `0` means "culled"; the GPU skips the draw entirely.
    pub instance_count: u32,
    /// First vertex — always 0; the shader offsets into the pool itself.
    pub first_vertex: u32,
    /// Slot in the instance buffer (`@builtin(instance_index)` in the shader).
    pub first_instance: u32,
}

/// Size of one `DrawIndirectArgs` in bytes.
pub const ARGS_SIZE: u64 = std::mem::size_of::<DrawIndirectArgs>() as u64;

/// A growable GPU buffer of indirect draw arguments, rebuilt each frame.
pub struct IndirectDrawBuffer {
    /// The GPU buffer (`INDIRECT | STORAGE | COPY_DST`).
    ///
    /// `STORAGE` is included now so the Phase 15B culling compute shader can
    /// write into it without reallocating.
    pub buffer: wgpu::Buffer,
    /// Capacity in draws.
    capacity: usize,
    /// Draws written by the most recent [`Self::update`].
    len: usize,
    /// CPU-side staging, reused across frames.
    staging: Vec<DrawIndirectArgs>,
}

/// Starting capacity in draws. The visibility buffer's 10-bit instance ID caps
/// the scene at 1022 draws today (lifted in Phase 15C), so this covers it.
const INITIAL_CAPACITY: usize = 1024;

impl IndirectDrawBuffer {
    /// Create the buffer with room for [`INITIAL_CAPACITY`] draws.
    pub fn new(device: &wgpu::Device) -> Self {
        Self {
            buffer: Self::alloc(device, INITIAL_CAPACITY),
            capacity: INITIAL_CAPACITY,
            len: 0,
            staging: Vec::with_capacity(INITIAL_CAPACITY),
        }
    }

    fn alloc(device: &wgpu::Device, capacity: usize) -> wgpu::Buffer {
        device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Indirect Draw Args"),
            size: capacity as u64 * ARGS_SIZE,
            usage: wgpu::BufferUsages::INDIRECT
                | wgpu::BufferUsages::STORAGE
                | wgpu::BufferUsages::COPY_DST
                // Lets the cull-stats diagnostic read back what each phase
                // decided. Costs nothing when the diagnostic is off.
                | wgpu::BufferUsages::COPY_SRC,
            mapped_at_creation: false,
        })
    }

    /// Number of draws in the buffer after the last [`Self::update`].
    pub fn len(&self) -> usize {
        self.len
    }

    /// Whether the buffer holds no draws.
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// Upload a pre-built argument list (Phase 15F builds one arg per cluster).
    pub fn upload(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        args: &[DrawIndirectArgs],
    ) {
        self.len = args.len();
        if self.len > self.capacity {
            let mut cap = self.capacity.max(1);
            while cap < self.len {
                cap *= 2;
            }
            self.buffer = Self::alloc(device, cap);
            self.capacity = cap;
        }
        if !args.is_empty() {
            queue.write_buffer(&self.buffer, 0, bytemuck::cast_slice(args));
        }
    }

    /// Rebuild the argument list from this frame's draw queue and upload it.
    ///
    /// Argument `i` corresponds to instance `i`, matching the order the
    /// instance buffer is built in, so `first_instance = i`.
    pub fn update(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, draws: &[DrawCommand]) {
        self.staging.clear();
        self.staging
            .extend(draws.iter().enumerate().map(|(i, cmd)| DrawIndirectArgs {
                vertex_count: cmd.index_count,
                instance_count: 1,
                first_vertex: 0,
                first_instance: i as u32,
            }));
        self.len = self.staging.len();

        if self.len > self.capacity {
            let mut cap = self.capacity.max(1);
            while cap < self.len {
                cap *= 2;
            }
            self.buffer = Self::alloc(device, cap);
            self.capacity = cap;
        }

        if !self.staging.is_empty() {
            queue.write_buffer(&self.buffer, 0, bytemuck::cast_slice(&self.staging));
        }
    }
}

/// Append the indirect arguments and cull bounds for one draw (Phase 15F).
///
/// With clusters, a draw becomes one argument per cluster so culling can work
/// below whole-object granularity. `first_vertex` is the cluster's index offset
/// *within its mesh*, because the vertex shader adds `instance.index_offset`
/// itself — and `@builtin(vertex_index)` includes `first_vertex`, which is also
/// how the shader recovers a mesh-relative triangle id.
///
/// Without clusters — voxel chunks, or when the meshlet path is off — the draw
/// stays a single whole-mesh argument, so both paths flow through one pipeline.
pub fn push_cluster_args(
    instance_index: u32,
    index_count: u32,
    meshlets: Option<&[Meshlet]>,
    fallback_bounds: Option<([f32; 3], [f32; 3])>,
    args: &mut Vec<DrawIndirectArgs>,
    bounds: &mut Vec<GpuCullAabb>,
) {
    match meshlets {
        Some(list) if !list.is_empty() => {
            for m in list {
                args.push(DrawIndirectArgs {
                    vertex_count: m.index_count(),
                    instance_count: 1,
                    first_vertex: m.index_offset(),
                    first_instance: instance_index,
                });
                // The cluster's own AABB, not its bounding sphere's. The
                // sphere is up to sqrt(3) wider per axis and can reach outside
                // the parent mesh's bounds, which let boundary clusters survive
                // frustum tests the whole mesh failed — measured as the cluster
                // path submitting *more* geometry than whole-mesh draws.
                bounds.push(GpuCullAabb {
                    min: [m.aabb_min[0], m.aabb_min[1], m.aabb_min[2], 0.0],
                    max: [m.aabb_max[0], m.aabb_max[1], m.aabb_max[2], 0.0],
                    cone: [
                        m.cone_axis[0],
                        m.cone_axis[1],
                        m.cone_axis[2],
                        m.backface_cutoff(),
                    ],
                });
            }
        }
        _ => {
            args.push(DrawIndirectArgs {
                vertex_count: index_count,
                instance_count: 1,
                first_vertex: 0,
                first_instance: instance_index,
            });
            bounds.push(match fallback_bounds {
                Some((min, max)) => GpuCullAabb::from_aabb(min, max),
                None => GpuCullAabb::never_culled(),
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::command::{DrawCommand, SortKey};

    fn draw(index_count: u32) -> DrawCommand {
        DrawCommand {
            casts_shadow: true,
            sort_key: SortKey::new(0, 0, 0),
            vertex_offset: 0,
            index_offset: 0,
            index_count,
            material_id: 0,
            transform: glam::Mat4::IDENTITY,
        }
    }

    /// Mirrors `IndirectDrawBuffer::update`'s CPU half so the mapping from draw
    /// commands to arguments can be checked without a GPU device.
    fn build(draws: &[DrawCommand]) -> Vec<DrawIndirectArgs> {
        draws
            .iter()
            .enumerate()
            .map(|(i, cmd)| DrawIndirectArgs {
                vertex_count: cmd.index_count,
                instance_count: 1,
                first_vertex: 0,
                first_instance: i as u32,
            })
            .collect()
    }

    #[test]
    fn args_match_the_draw_queue_order() {
        let draws = [draw(36), draw(2298), draw(6)];
        let args = build(&draws);
        assert_eq!(args.len(), 3);
        for (i, (a, d)) in args.iter().zip(&draws).enumerate() {
            // first_instance must equal the draw's index, or the vertex shader
            // reads the wrong instance's transform.
            assert_eq!(a.first_instance, i as u32);
            assert_eq!(a.vertex_count, d.index_count);
            assert_eq!(a.instance_count, 1, "draws start visible");
            assert_eq!(a.first_vertex, 0);
        }
    }

    #[test]
    fn args_struct_is_the_16_byte_gpu_layout() {
        // wgpu reads these straight out of the buffer; a size change would
        // silently misalign every draw.
        assert_eq!(ARGS_SIZE, 16);
        assert_eq!(std::mem::align_of::<DrawIndirectArgs>(), 4);
    }

    #[test]
    fn empty_queue_produces_no_args() {
        assert!(build(&[]).is_empty());
    }

    // ── Phase 15F: per-cluster arguments ────────────────────────────────────

    fn meshlet(offset: u32, count: u32) -> Meshlet {
        Meshlet {
            triangle_offset: offset,
            triangle_count: count,
            center: [0.0, 0.0, 0.0],
            radius: 2.0,
            aabb_min: [-1.0, -1.0, -1.0],
            aabb_max: [1.0, 1.0, 1.0],
            cone_axis: [0.0, 1.0, 0.0],
            cone_cutoff: 1.0,
        }
    }

    #[test]
    fn each_cluster_becomes_its_own_draw() {
        let (mut args, mut bounds) = (Vec::new(), Vec::new());
        let list = [meshlet(0, 128), meshlet(128, 128), meshlet(256, 44)];
        push_cluster_args(7, 900, Some(&list), None, &mut args, &mut bounds);

        assert_eq!(args.len(), 3);
        assert_eq!(bounds.len(), 3, "bounds must stay parallel to args");
        for a in &args {
            // Every cluster of a draw shades with the same instance.
            assert_eq!(a.first_instance, 7);
            assert_eq!(a.instance_count, 1);
        }
        // first_vertex is mesh-relative in indices, not triangles.
        assert_eq!(args[0].first_vertex, 0);
        assert_eq!(args[1].first_vertex, 128 * 3);
        assert_eq!(args[2].first_vertex, 256 * 3);
        assert_eq!(args[2].vertex_count, 44 * 3);
    }

    #[test]
    fn clusters_cover_the_whole_mesh_exactly_once() {
        let (mut args, mut bounds) = (Vec::new(), Vec::new());
        let list = [meshlet(0, 128), meshlet(128, 128), meshlet(256, 44)];
        push_cluster_args(0, 900, Some(&list), None, &mut args, &mut bounds);
        // No gaps and no overlap, or triangles would be dropped or drawn twice.
        let mut next = 0;
        for a in &args {
            assert_eq!(a.first_vertex, next);
            next += a.vertex_count;
        }
        assert_eq!(next, 900);
    }

    #[test]
    fn a_mesh_without_clusters_stays_one_draw() {
        let (mut args, mut bounds) = (Vec::new(), Vec::new());
        push_cluster_args(
            3,
            600,
            None,
            Some(([-1.0; 3], [1.0; 3])),
            &mut args,
            &mut bounds,
        );
        assert_eq!(args.len(), 1);
        assert_eq!(args[0].vertex_count, 600);
        assert_eq!(args[0].first_vertex, 0);
        assert_eq!(args[0].first_instance, 3);
        // Cone culling must not fire on a whole mesh: its normals point everywhere.
        assert_eq!(bounds[0].cone[3], 2.0);
    }

    #[test]
    fn an_empty_cluster_list_falls_back_to_a_whole_mesh_draw() {
        let (mut args, mut bounds) = (Vec::new(), Vec::new());
        push_cluster_args(0, 300, Some(&[]), None, &mut args, &mut bounds);
        assert_eq!(args.len(), 1);
        assert_eq!(args[0].vertex_count, 300);
        assert_eq!(bounds[0].min[0], f32::MIN, "unknown bounds must never cull");
    }

    #[test]
    fn cluster_bounds_use_the_box_not_the_sphere() {
        let (mut args, mut bounds) = (Vec::new(), Vec::new());
        let mut m = meshlet(0, 10);
        m.aabb_min = [2.0, -5.0, -2.0];
        m.aabb_max = [8.0, 1.0, 4.0];
        push_cluster_args(0, 30, Some(&[m]), None, &mut args, &mut bounds);
        assert_eq!(bounds[0].min[0], 2.0);
        assert_eq!(bounds[0].max[0], 8.0);
        assert_eq!(bounds[0].min[1], -5.0);
        assert_eq!(bounds[0].max[2], 4.0);
    }
}
