//! Instance batches: many copies of the same mesh parts, kept on the GPU between frames (open-world render scale,
//! 18-19 October 2026).
//!
//! A draw given to [`crate::SomniumRenderer::submit`] costs the CPU every frame: it is sorted, turned into an
//! instance, a motion record, an indirect argument and a shadow-caster test, about 0.15 µs in all at 300,000 draws
//! (`scale_probe`). A region's foliage cannot pay that: a million copies would be 150 ms a frame. A batch is set
//! once, with every placement of its parts, and stays. Each copy of each part is a *slot* in a GPU buffer. Every frame
//! one compute dispatch (`instance_batch.wgsl` `expand`) writes the slots' indirect arguments after the frame's own,
//! where the existing two-phase cull (frustum, the slot's distance band, Hi-Z) and the visibility pass take them
//! like any other draw. A second dispatch (`shadow_cull`) finds the shadow casters in each cascade being redrawn. The
//! CPU pays per batch set or removed, not per copy per frame.
//!
//! Ray tracing: the copies within 60 m of the eye whose mesh has a bottom level join the top level each frame
//! (nearest first, at most 2,048, after every submitted draw), so near foliage still shadows traced lights, bounces
//! light and shows in a reflection. Farther copies are raster-only.
//!
//! What a batch does not have, so what stays with `submit`: ray tracing past that reach, the mesh SDF, virtual shadow pages (cascades only), motion of its own (copies stand still; wind still
//! sways them), vertex paint, the game and editor highlights, reflection captures, and back-face culling: batches
//! draw two-sided, as foliage does.

use bytemuck::{Pod, Zeroable};
use glam::Mat4;

/// One mesh part every placement of a batch draws: a tree's trunk, its leaves, a far LOD.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BatchPart {
    pub vertex_offset: u32,
    pub index_offset: u32,
    pub index_count: u32,
    pub material_id: u32,
    /// The part's transform within one placement.
    pub local: Mat4,
    /// Drawn from this horizontal distance to the camera, in metres (0: from the eye).
    pub near: f32,
    /// Drawn until this distance (0: no limit). A LOD switch is two parts, one's `far` the other's `near`.
    pub far: f32,
    pub casts_shadow: bool,
    /// Casts while nearer than this (0: no limit).
    pub shadow_far: f32,
}

/// Every placement of a set of parts, drawn every frame until it is replaced or removed.
#[derive(Clone, Debug, Default)]
pub struct InstanceBatch {
    pub parts: Vec<BatchPart>,
    /// World transforms, one per copy.
    pub placements: Vec<Mat4>,
}

/// A copy whose mesh has a bottom level: what the top level needs to trace it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct TracedCopy {
    /// Its slot within its batch.
    pub slot: u32,
    pub vertex_offset: u32,
    pub model: Mat4,
    /// It stops shadow rays (the renderer's `traces_shadow`).
    pub shadow: bool,
    /// Its part's horizontal draw band: traced only where it is drawn.
    pub near: f32,
    pub far: f32,
}

/// A batch's traced copies and the horizontal box their origins span.
struct Traced {
    start: u32,
    copies: Vec<TracedCopy>,
    lo: [f32; 2],
    hi: [f32; 2],
}

/// One copy of one part, as `instance_batch.wgsl` reads it.
#[repr(C)]
#[derive(Copy, Clone, Debug, Pod, Zeroable)]
pub(crate) struct GpuSlot {
    pub model: [[f32; 4]; 4],
    pub material_id: u32,
    pub vertex_offset: u32,
    pub index_offset: u32,
    /// 0: a free slot.
    pub index_count: u32,
    /// Local bounds; `w`s are the far and near draw distances.
    pub aabb_min: [f32; 4],
    pub aabb_max: [f32; 4],
    /// Shadow distance, casts (0 or 1).
    pub shadow: [f32; 4],
}

const SLOT_SIZE: u64 = std::mem::size_of::<GpuSlot>() as u64;

impl GpuSlot {
    /// Slot for `part` placed at `placement`, with the mesh's local `bounds` (`None`: never culled).
    pub(crate) fn new(part: &BatchPart, placement: Mat4, bounds: Option<([f32; 3], [f32; 3])>) -> Self {
        let (min, max) = bounds.unwrap_or(([f32::MIN; 3], [f32::MAX; 3]));
        Self {
            model: (placement * part.local).to_cols_array_2d(),
            material_id: part.material_id,
            vertex_offset: part.vertex_offset,
            index_offset: part.index_offset,
            index_count: part.index_count,
            aabb_min: [min[0], min[1], min[2], part.far.max(0.0)],
            aabb_max: [max[0], max[1], max[2], part.near.max(0.0)],
            shadow: [part.shadow_far.max(0.0), f32::from(u8::from(part.casts_shadow)), 0.0, 0.0],
        }
    }
}

/// Slot ranges: first fit, merged with their neighbours when freed, the end given back when the last range goes.
#[derive(Default, Debug)]
struct SlotRanges {
    /// `(start, len)`, by start.
    free: Vec<(u32, u32)>,
    end: u32,
}

impl SlotRanges {
    fn alloc(&mut self, len: u32) -> u32 {
        if let Some(k) = self.free.iter().position(|&(_, free)| free >= len) {
            let (start, free) = self.free[k];
            if free == len {
                self.free.remove(k);
            } else {
                self.free[k] = (start + len, free - len);
            }
            return start;
        }
        let start = self.end;
        self.end += len;
        start
    }

    fn release(&mut self, start: u32, len: u32) {
        if len == 0 {
            return;
        }
        let k = self.free.partition_point(|&(s, _)| s < start);
        self.free.insert(k, (start, len));
        if k + 1 < self.free.len() && self.free[k].0 + self.free[k].1 == self.free[k + 1].0 {
            self.free[k].1 += self.free[k + 1].1;
            self.free.remove(k + 1);
        }
        if k > 0 && self.free[k - 1].0 + self.free[k - 1].1 == self.free[k].0 {
            self.free[k - 1].1 += self.free[k].1;
            self.free.remove(k);
        }
        if let Some(&(s, l)) = self.free.last()
            && s + l == self.end
        {
            self.end = s;
            self.free.pop();
        }
    }
}

enum Write {
    Slots(u32, Vec<GpuSlot>),
    Free(u32, u32),
}

/// Smallest start for the batch region in the instance and argument buffers. The region sits past the frame's own
/// instances and moves (rewriting every slot's instance and bounds) only when the frame outgrows it, so it starts
/// past what most scenes submit.
const MIN_BASE: u32 = 1 << 16;

/// Where this frame's batch instances and arguments go, past `frame_len` of the frame's own and never lower than
/// last frame's start: an index the frame's own per-draw arrays wrote is always below it (`instance_batch.rs`).
#[must_use]
pub(crate) fn region_start(frame_len: usize, last: u32) -> u32 {
    let need = u32::try_from(frame_len).unwrap_or(u32::MAX / 2).max(1).next_power_of_two();
    need.max(MIN_BASE).max(last)
}

struct BatchGpu {
    expand_layout: wgpu::BindGroupLayout,
    shadow_layout: wgpu::BindGroupLayout,
    expand_pipeline: wgpu::ComputePipeline,
    shadow_pipeline: wgpu::ComputePipeline,
    expand_params: wgpu::Buffer,
    shadow_params: wgpu::Buffer,
    slots: wgpu::Buffer,
    capacity: u32,
    shadow_args: wgpu::Buffer,
    shadow_counts: wgpu::Buffer,
}

#[repr(C)]
#[derive(Copy, Clone, Pod, Zeroable)]
struct ExpandParams {
    slots: u32,
    instance_base: u32,
    arg_base: u32,
    write_static: u32,
}

#[repr(C)]
#[derive(Copy, Clone, Pod, Zeroable)]
struct ShadowParams {
    planes: [[f32; 4]; 24],
    camera: [f32; 4],
    slots: u32,
    capacity: u32,
    dirty: u32,
    compact: u32,
    instance_base: u32,
    radius_threshold: f32,
    _pad: [u32; 2],
}

fn storage(binding: u32, read_only: bool) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::COMPUTE,
        ty: wgpu::BindingType::Buffer {
            ty: wgpu::BufferBindingType::Storage { read_only },
            has_dynamic_offset: false,
            min_binding_size: None,
        },
        count: None,
    }
}

fn uniform(binding: u32) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::COMPUTE,
        ty: wgpu::BindingType::Buffer {
            ty: wgpu::BufferBindingType::Uniform,
            has_dynamic_offset: false,
            min_binding_size: None,
        },
        count: None,
    }
}

fn slot_buffer(device: &wgpu::Device, capacity: u32) -> wgpu::Buffer {
    device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("Instance batch slots"),
        size: u64::from(capacity.max(1)) * SLOT_SIZE,
        usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::COPY_SRC,
        mapped_at_creation: false,
    })
}

fn shadow_args_buffer(device: &wgpu::Device, capacity: u32) -> wgpu::Buffer {
    device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("Instance batch shadow draws"),
        size: 4 * u64::from(capacity.max(1)) * crate::indirect::ARGS_SIZE,
        // COPY_SRC: read back by the tests.
        usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::INDIRECT | wgpu::BufferUsages::COPY_SRC,
        mapped_at_creation: false,
    })
}

impl BatchGpu {
    fn new(device: &wgpu::Device, shaders: &crate::shaders::Shaders) -> Self {
        let expand_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("Instance batch expand layout"),
            entries: &[storage(0, true), uniform(1), storage(2, false), storage(3, false), storage(4, false)],
        });
        let shadow_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("Instance batch shadow layout"),
            entries: &[storage(0, true), uniform(5), storage(6, false), storage(7, false)],
        });
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("Instance batch shader"),
            source: wgpu::ShaderSource::Wgsl(shaders.source_or_panic("instance_batch.wgsl").into()),
        });
        let pipeline = |layout: &wgpu::BindGroupLayout, entry: &str| {
            let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("Instance batch pipeline layout"),
                bind_group_layouts: &[Some(layout)],
                immediate_size: 0,
            });
            device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                label: Some(entry),
                layout: Some(&pipeline_layout),
                module: &module,
                entry_point: Some(entry),
                compilation_options: Default::default(),
                cache: None,
            })
        };
        let expand_pipeline = pipeline(&expand_layout, "expand");
        let shadow_pipeline = pipeline(&shadow_layout, "shadow_cull");
        let params = |label, size| {
            device.create_buffer(&wgpu::BufferDescriptor {
                label: Some(label),
                size,
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            })
        };
        let capacity = 1024;
        Self {
            expand_params: params("Instance batch expand params", std::mem::size_of::<ExpandParams>() as u64),
            shadow_params: params("Instance batch shadow params", std::mem::size_of::<ShadowParams>() as u64),
            slots: slot_buffer(device, capacity),
            capacity,
            shadow_args: shadow_args_buffer(device, capacity),
            shadow_counts: device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("Instance batch shadow counts"),
                size: 16,
                usage: wgpu::BufferUsages::STORAGE
                    | wgpu::BufferUsages::INDIRECT
                    | wgpu::BufferUsages::COPY_DST
                    | wgpu::BufferUsages::COPY_SRC,
                mapped_at_creation: false,
            }),
            expand_layout,
            shadow_layout,
            expand_pipeline,
            shadow_pipeline,
        }
    }
}

/// This frame's batch shadow draws, for [`crate::pass::shadow::ShadowPass::record_with_batches`].
pub struct BatchShadowDraws<'a> {
    pub args: &'a wgpu::Buffer,
    /// Each cascade's count (`None`: the dense list, a 0 or 1 count per slot).
    pub counts: Option<&'a wgpu::Buffer>,
    /// Each cascade's region of `args`, in arguments.
    pub capacity: u32,
    /// Slots to draw at most.
    pub slots: u32,
}

/// The renderer's instance batches.
#[derive(Default)]
pub(crate) struct InstanceBatches {
    batches: crate::fast_hash::FastMap<u64, (u32, u32)>,
    traced: crate::fast_hash::FastMap<u64, Traced>,
    ranges: SlotRanges,
    pending: Vec<Write>,
    revision: u64,
    gpu: Option<BatchGpu>,
    /// What the instance and bounds regions hold: (revision, instance start, argument start).
    written: Option<(u64, u32, u32)>,
    pub(crate) instance_base: u32,
    pub(crate) arg_base: u32,
    /// The shadow cull ran this frame, into dense (false) or counted (true) draws.
    shadow_culled: Option<bool>,
}

impl InstanceBatches {
    /// Replace batch `id` with `slots` (empty removes it), `traced` naming those of them the top level may trace.
    /// Uploaded on the next [`Self::flush`].
    pub(crate) fn set(&mut self, id: u64, slots: Vec<GpuSlot>, traced: Vec<TracedCopy>) {
        self.remove(id);
        if slots.is_empty() {
            return;
        }
        let len = u32::try_from(slots.len()).unwrap_or(u32::MAX);
        let start = self.ranges.alloc(len);
        self.batches.insert(id, (start, len));
        if !traced.is_empty() {
            let (mut lo, mut hi) = ([f32::MAX; 2], [f32::MIN; 2]);
            for copy in &traced {
                let at = copy.model.w_axis;
                lo = [lo[0].min(at.x), lo[1].min(at.z)];
                hi = [hi[0].max(at.x), hi[1].max(at.z)];
            }
            self.traced.insert(id, Traced { start, copies: traced, lo, hi });
        }
        self.pending.push(Write::Slots(start, slots));
        self.revision = self.revision.wrapping_add(1);
    }

    pub(crate) fn remove(&mut self, id: u64) -> bool {
        let Some((start, len)) = self.batches.remove(&id) else {
            return false;
        };
        self.traced.remove(&id);
        self.ranges.release(start, len);
        self.pending.push(Write::Free(start, len));
        self.revision = self.revision.wrapping_add(1);
        true
    }

    pub(crate) fn clear(&mut self) {
        let ids: Vec<u64> = self.batches.keys().copied().collect();
        for id in ids {
            self.remove(id);
        }
    }

    /// No slot is live. A renderer that never set a batch stays here, and every batch step is skipped.
    pub(crate) fn is_empty(&self) -> bool {
        self.ranges.end == 0
    }

    /// Slots in use or free below the last in use: what every dispatch and draw covers.
    pub(crate) fn slot_end(&self) -> u32 {
        self.ranges.end
    }

    /// Changes with every set and remove (the shadow cascades are redrawn on it).
    pub(crate) fn revision(&self) -> u64 {
        self.revision
    }

    pub(crate) fn batch_count(&self) -> usize {
        self.batches.len()
    }

    /// Send what was set or removed since the last frame, growing the slot buffer first if it must.
    pub(crate) fn flush(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, shaders: &crate::shaders::Shaders) {
        if self.pending.is_empty() {
            return;
        }
        let gpu = self.gpu.get_or_insert_with(|| BatchGpu::new(device, shaders));
        let end = self.ranges.end.max(self.pending.iter().map(|w| match w {
            Write::Slots(start, slots) => start + slots.len() as u32,
            Write::Free(start, len) => start + len,
        }).max().unwrap_or(0));
        if end > gpu.capacity {
            let capacity = end.next_power_of_two();
            let grown = slot_buffer(device, capacity);
            // Submitted now, before this frame's slot writes: a queue write runs at the start of the next submit,
            // so on the frame's own encoder the copy would land after the writes and undo them.
            let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("Instance batch slots grow"),
            });
            encoder.copy_buffer_to_buffer(&gpu.slots, 0, &grown, 0, u64::from(gpu.capacity) * SLOT_SIZE);
            queue.submit([encoder.finish()]);
            gpu.slots = grown;
            gpu.capacity = capacity;
            gpu.shadow_args = shadow_args_buffer(device, capacity);
        }
        for write in self.pending.drain(..) {
            match write {
                Write::Slots(start, slots) => {
                    queue.write_buffer(&gpu.slots, u64::from(start) * SLOT_SIZE, bytemuck::cast_slice(&slots));
                }
                Write::Free(start, len) => {
                    let zero = vec![GpuSlot::zeroed(); len as usize];
                    queue.write_buffer(&gpu.slots, u64::from(start) * SLOT_SIZE, bytemuck::cast_slice(&zero));
                }
            }
        }
    }

    /// Write every slot's argument at `arg_base`, and, when the slots or either start changed or `force` (a buffer
    /// was reallocated), every slot's instance at `instance_base` and bounds at `arg_base`.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn record_expand(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        instances: &wgpu::Buffer,
        args: &wgpu::Buffer,
        aabbs: &wgpu::Buffer,
        force: bool,
    ) {
        self.shadow_culled = None;
        let Some(gpu) = self.gpu.as_ref() else { return };
        let slots = self.ranges.end;
        if slots == 0 {
            return;
        }
        let key = (self.revision, self.instance_base, self.arg_base);
        let write_static = force || self.written != Some(key);
        self.written = Some(key);
        let params = ExpandParams {
            slots,
            instance_base: self.instance_base,
            arg_base: self.arg_base,
            write_static: u32::from(write_static),
        };
        queue.write_buffer(&gpu.expand_params, 0, bytemuck::bytes_of(&params));
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("Instance batch expand"),
            layout: &gpu.expand_layout,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: gpu.slots.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 1, resource: gpu.expand_params.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 2, resource: instances.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 3, resource: args.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 4, resource: aabbs.as_entire_binding() },
            ],
        });
        let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
            label: Some("Instance batch expand"),
            timestamp_writes: None,
        });
        pass.set_pipeline(&gpu.expand_pipeline);
        pass.set_bind_group(0, &bind_group, &[]);
        pass.dispatch_workgroups(slots.div_ceil(64), 1, 1);
    }

    /// Cull the shadow-casting slots into each cascade in `dirty` (planes as `culling::frustum_planes`), leaving
    /// out copies too small at their distance for `radius_threshold` (0: none), as submitted casters are.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn record_shadow_cull(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        planes: &[[[f32; 4]; 6]; crate::shadow::NUM_CASCADES],
        camera: glam::Vec3,
        dirty: [bool; crate::shadow::NUM_CASCADES],
        compact: bool,
        radius_threshold: f32,
    ) {
        self.shadow_culled = None;
        let Some(gpu) = self.gpu.as_ref() else { return };
        let slots = self.ranges.end;
        let mask = dirty.iter().enumerate().fold(0u32, |m, (i, d)| m | (u32::from(*d) << i));
        if slots == 0 || mask == 0 {
            return;
        }
        let mut flat = [[0.0f32; 4]; 24];
        for (c, cascade) in planes.iter().enumerate().take(4) {
            flat[c * 6..c * 6 + 6].copy_from_slice(cascade);
        }
        let params = ShadowParams {
            planes: flat,
            camera: [camera.x, camera.y, camera.z, 0.0],
            slots,
            capacity: gpu.capacity,
            dirty: mask,
            compact: u32::from(compact),
            instance_base: self.instance_base,
            radius_threshold,
            _pad: [0; 2],
        };
        queue.write_buffer(&gpu.shadow_params, 0, bytemuck::bytes_of(&params));
        if compact {
            encoder.clear_buffer(&gpu.shadow_counts, 0, None);
        }
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("Instance batch shadow cull"),
            layout: &gpu.shadow_layout,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: gpu.slots.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 5, resource: gpu.shadow_params.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 6, resource: gpu.shadow_args.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 7, resource: gpu.shadow_counts.as_entire_binding() },
            ],
        });
        let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
            label: Some("Instance batch shadow cull"),
            timestamp_writes: None,
        });
        pass.set_pipeline(&gpu.shadow_pipeline);
        pass.set_bind_group(0, &bind_group, &[]);
        pass.dispatch_workgroups(slots.div_ceil(64), 1, 1);
        self.shadow_culled = Some(compact);
    }

    /// The traced copies within `reach` horizontal metres of `eye` whose part is drawn at that distance, as the top
    /// level's `(instance index, vertex offset, model, stops shadow rays)`, appended to `out`.
    pub(crate) fn traced_near(&self, eye: glam::Vec3, reach: f32, out: &mut Vec<(u32, u32, Mat4, bool)>) {
        let (ex, ez) = (eye.x, eye.z);
        for traced in self.traced.values() {
            let dx = (traced.lo[0] - ex).max(ex - traced.hi[0]).max(0.0);
            let dz = (traced.lo[1] - ez).max(ez - traced.hi[1]).max(0.0);
            if dx * dx + dz * dz > reach * reach {
                continue;
            }
            for copy in &traced.copies {
                let at = copy.model.w_axis;
                let d2 = (at.x - ex) * (at.x - ex) + (at.z - ez) * (at.z - ez);
                if d2 > reach * reach || (copy.far > 0.0 && d2 > copy.far * copy.far) || d2 < copy.near * copy.near {
                    continue;
                }
                out.push((self.instance_base + traced.start + copy.slot, copy.vertex_offset, copy.model, copy.shadow));
            }
        }
    }

    /// The shadow draws [`Self::record_shadow_cull`] made this frame, if it ran.
    pub(crate) fn shadow_draws(&self) -> Option<BatchShadowDraws<'_>> {
        let compact = self.shadow_culled?;
        let gpu = self.gpu.as_ref()?;
        Some(BatchShadowDraws {
            args: &gpu.shadow_args,
            counts: compact.then_some(&gpu.shadow_counts),
            capacity: gpu.capacity,
            slots: self.ranges.end,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_slot_is_what_the_shader_reads() {
        assert_eq!(SLOT_SIZE, 128);
        assert_eq!(std::mem::size_of::<ShadowParams>() % 16, 0);
        assert_eq!(std::mem::size_of::<ShadowParams>(), 432);
    }

    #[test]
    fn freed_ranges_are_reused_merged_and_given_back_at_the_end() {
        let mut r = SlotRanges::default();
        let a = r.alloc(10);
        let b = r.alloc(5);
        let c = r.alloc(7);
        assert_eq!((a, b, c, r.end), (0, 10, 15, 22));
        r.release(b, 5);
        // Too big for the hole: past the end.
        assert_eq!(r.alloc(6), 22);
        // Fits: first fit.
        assert_eq!(r.alloc(3), 10);
        r.release(a, 10);
        r.release(10, 3);
        // 0..15 is one hole now.
        assert_eq!(r.free, vec![(0, 15)]);
        r.release(c, 7);
        r.release(22, 6);
        assert_eq!((r.end, r.free.len()), (0, 0));
    }

    #[test]
    fn the_region_starts_past_the_frame_and_never_moves_down() {
        assert_eq!(region_start(10, 0), MIN_BASE);
        assert_eq!(region_start(100_000, 0), 131_072);
        assert_eq!(region_start(70_000, 131_072), 131_072);
        assert_eq!(region_start(131_073, 131_072), 262_144);
    }

    #[test]
    fn replacing_a_batch_frees_its_old_slots() {
        let mut batches = InstanceBatches::default();
        let part = BatchPart {
            vertex_offset: 0,
            index_offset: 0,
            index_count: 3,
            material_id: 0,
            local: Mat4::IDENTITY,
            near: 0.0,
            far: 0.0,
            casts_shadow: true,
            shadow_far: 0.0,
        };
        let slot = GpuSlot::new(&part, Mat4::IDENTITY, Some(([0.0; 3], [1.0; 3])));
        batches.set(7, vec![slot; 100], Vec::new());
        batches.set(8, vec![slot; 50], Vec::new());
        assert_eq!(batches.slot_end(), 150);
        batches.set(7, vec![slot; 20], Vec::new());
        // The first 20 of 7's old range, then 8's.
        assert_eq!(batches.batches[&7], (0, 20));
        assert_eq!(batches.slot_end(), 150);
        assert!(batches.remove(8));
        assert_eq!(batches.slot_end(), 20);
        batches.clear();
        assert!(batches.is_empty());
        assert_eq!(batches.batch_count(), 0);
    }

    #[test]
    fn near_copies_are_traced_where_they_are_drawn() {
        let mut batches = InstanceBatches::default();
        let part = BatchPart {
            vertex_offset: 4,
            index_offset: 0,
            index_count: 3,
            material_id: 0,
            local: Mat4::IDENTITY,
            near: 0.0,
            far: 0.0,
            casts_shadow: true,
            shadow_far: 0.0,
        };
        let copy = |slot: u32, x: f32, near: f32, far: f32| TracedCopy {
            slot,
            vertex_offset: 4,
            model: Mat4::from_translation(glam::Vec3::new(x, 0.0, 0.0)),
            shadow: true,
            near,
            far,
        };
        let slot = GpuSlot::new(&part, Mat4::IDENTITY, None);
        // One batch near the eye: a copy at 5 m, a far LOD (from 20 m) at 5 m, one at 80 m; a second batch 500 m away.
        batches.set(1, vec![slot; 4], vec![copy(0, 5.0, 0.0, 0.0), copy(1, 5.0, 20.0, 0.0), copy(3, 80.0, 0.0, 0.0)]);
        batches.set(2, vec![slot; 2], vec![copy(0, 500.0, 0.0, 0.0)]);
        batches.instance_base = 1000;
        let mut out = Vec::new();
        batches.traced_near(glam::Vec3::ZERO, 60.0, &mut out);
        let indices: Vec<u32> = out.iter().map(|t| t.0).collect();
        assert_eq!(indices, [1000], "only the near copy drawn at its distance, at its instance");
        batches.remove(1);
        out.clear();
        batches.traced_near(glam::Vec3::new(500.0, 0.0, 0.0), 60.0, &mut out);
        assert_eq!(out.iter().map(|t| t.0).collect::<Vec<_>>(), [1004], "batch 2's slot after batch 1's four");
    }
}

#[cfg(test)]
#[path = "instance_batch_tests.rs"]
mod gpu_tests;
