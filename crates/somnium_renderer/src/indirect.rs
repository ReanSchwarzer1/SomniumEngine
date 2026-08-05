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

    /// Rebuild the argument list from this frame's draw queue and upload it.
    ///
    /// Argument `i` corresponds to instance `i`, matching the order the
    /// instance buffer is built in, so `first_instance = i`.
    pub fn update(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, draws: &[DrawCommand]) {
        self.staging.clear();
        self.staging.extend(draws.iter().enumerate().map(|(i, cmd)| DrawIndirectArgs {
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::command::{DrawCommand, SortKey};

    fn draw(index_count: u32) -> DrawCommand {
        DrawCommand {
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
}
