//! MORROWIND-U — the skinning compute pass.
//!
//! One dispatch per frame, before culling. It reads each skinned instance's
//! rest vertices out of [`GeometryPool`](crate::geometry::GeometryPool)'s
//! shared vertex buffer, applies the instance's palette, and writes the posed
//! vertices back into a reserved span of the **same** buffer.
//!
//! That last part is the whole design. Every consumer downstream — culling,
//! Hi-Z, the visibility pass, ray tracing — keeps reading the one buffer it
//! always read, and none of them learns that skinning exists. The reasoning,
//! and what the alternative would have cost, is in
//! [`crate::skinning`](crate::skinning).
//!
//! # Where it goes in the frame
//!
//! **Before culling**, because culling tests bounds and the bounds of a posed
//! mesh are only known once it is posed. `SkinningPalettes::posed_bounds` gives
//! the conservative box on the CPU from the palette, so the two do not have to
//! be sequenced on the GPU — but the *vertices* do, and a cull pass reading a
//! half-written posed span would flicker.

use crate::skinning::{SkinInstance, SkinVertex, SkinningPalettes, WORKGROUP_SIZE};

/// GPU state for the skinning dispatch.
pub struct SkinPass {
    pipeline: wgpu::ComputePipeline,
    layout: wgpu::BindGroupLayout,
    /// One `mat4x4<f32>` per joint per instance.
    palettes: wgpu::Buffer,
    /// Per-vertex bindings, parallel to the pool's vertices.
    skin_vertices: wgpu::Buffer,
    /// What to skin and where to put it.
    instances: wgpu::Buffer,
    bind_group: Option<wgpu::BindGroup>,
    /// Capacities, in elements, so a growth is a reallocation rather than a
    /// silent truncation.
    palette_capacity: usize,
    instance_capacity: usize,
    skin_vertex_capacity: usize,
    /// Size of the pool buffer the current bind group was built against.
    ///
    /// `GeometryPool` reallocates when it grows, and a bind group holding the
    /// old buffer would skin into memory nothing reads — a character that
    /// simply stops animating, which is the hardest kind of bug to find because
    /// nothing errors. wgpu 30 removed `Buffer::global_id`, so identity is
    /// tracked by size: the pool only ever grows, so a different size is a
    /// different buffer and the same size is the same one.
    bound_pool_size: Option<u64>,
}

impl SkinPass {
    /// Elements to allocate before anything has registered. Small, because the
    /// overwhelming majority of scenes have no skinned meshes at all and should
    /// not pay for the possibility.
    const INITIAL_CAPACITY: usize = 256;

    #[must_use]
    pub fn new(device: &wgpu::Device, shaders: &crate::shaders::Shaders) -> Self {
        let source = shaders.source_or_panic("skinning.wgsl");
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("skinning.wgsl"),
            source: wgpu::ShaderSource::Wgsl(source.into()),
        });

        let storage = |binding: u32, read_only: bool| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::COMPUTE,
            ty: wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Storage { read_only },
                has_dynamic_offset: false,
                min_binding_size: None,
            },
            count: None,
        };

        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("Skin BGL"),
            entries: &[
                storage(0, true),  // palettes
                storage(1, true),  // skin vertices
                storage(2, true),  // instances
                storage(3, false), // the pool, read and written
            ],
        });

        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("Skin PL"),
            // Group 0 is unused by this shader — `global_pool.wgsl` is included
            // for its `Vertex` declaration and nothing else. An empty group 0
            // would be a layout mismatch, so the shader's only group is 1 and
            // the layout says so.
            bind_group_layouts: &[Some(&empty_layout(device)), Some(&layout)],
            immediate_size: 0,
        });

        let pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("Skin"),
            layout: Some(&pipeline_layout),
            module: &shader,
            entry_point: Some("skin"),
            compilation_options: wgpu::PipelineCompilationOptions::default(),
            cache: None,
        });

        Self {
            pipeline,
            layout,
            palettes: storage_buffer::<[f32; 16]>(device, "Skin palettes", Self::INITIAL_CAPACITY),
            skin_vertices: storage_buffer::<SkinVertex>(
                device,
                "Skin vertices",
                Self::INITIAL_CAPACITY,
            ),
            instances: storage_buffer::<SkinInstance>(
                device,
                "Skin instances",
                Self::INITIAL_CAPACITY,
            ),
            bind_group: None,
            palette_capacity: Self::INITIAL_CAPACITY,
            instance_capacity: Self::INITIAL_CAPACITY,
            skin_vertex_capacity: Self::INITIAL_CAPACITY,
            bound_pool_size: None,
        }
    }

    /// Upload a mesh's per-vertex bindings, once at bind time.
    ///
    /// `pool_offset` is the vertex offset the mesh's *rest* vertices live at,
    /// because the shader indexes `skin_vertices` with the same index it uses
    /// for the pool — one array, parallel to the whole pool, rather than one
    /// per mesh with an extra indirection.
    pub fn upload_bindings(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        pool_offset: u32,
        bindings: &[somnium_anim::SkinBinding],
    ) {
        let end = pool_offset as usize + bindings.len();
        if end > self.skin_vertex_capacity {
            self.skin_vertex_capacity = end.next_power_of_two();
            self.skin_vertices =
                storage_buffer::<SkinVertex>(device, "Skin vertices", self.skin_vertex_capacity);
            // The bind group holds the old buffer.
            self.bind_group = None;
        }
        let packed: Vec<SkinVertex> = bindings.iter().copied().map(SkinVertex::pack).collect();
        queue.write_buffer(
            &self.skin_vertices,
            pool_offset as u64 * std::mem::size_of::<SkinVertex>() as u64,
            bytemuck::cast_slice(&packed),
        );
    }

    /// Upload this frame's palettes and instances, and record the dispatch.
    ///
    /// A no-op when nothing is skinned, which is the common case and should
    /// cost one branch.
    pub fn record(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        pool: &wgpu::Buffer,
        palettes: &SkinningPalettes,
    ) {
        if palettes.is_empty() {
            return;
        }

        let matrices: Vec<[f32; 16]> = palettes
            .palette()
            .iter()
            .map(|m| m.to_cols_array())
            .collect();
        if matrices.len() > self.palette_capacity {
            self.palette_capacity = matrices.len().next_power_of_two();
            self.palettes =
                storage_buffer::<[f32; 16]>(device, "Skin palettes", self.palette_capacity);
            self.bind_group = None;
        }
        if palettes.instances().len() > self.instance_capacity {
            self.instance_capacity = palettes.instances().len().next_power_of_two();
            self.instances =
                storage_buffer::<SkinInstance>(device, "Skin instances", self.instance_capacity);
            self.bind_group = None;
        }

        queue.write_buffer(&self.palettes, 0, bytemuck::cast_slice(&matrices));
        queue.write_buffer(
            &self.instances,
            0,
            bytemuck::cast_slice(palettes.instances()),
        );

        if self.bound_pool_size != Some(pool.size()) {
            self.bind_group = None;
            self.bound_pool_size = Some(pool.size());
        }
        let bind_group = self.bind_group.get_or_insert_with(|| {
            device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("Skin BG"),
                layout: &self.layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: self.palettes.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: self.skin_vertices.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 2,
                        resource: self.instances.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 3,
                        resource: pool.as_entire_binding(),
                    },
                ],
            })
        });

        let (x, y, z) = palettes.dispatch();
        if x == 0 || y == 0 {
            return;
        }

        let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
            label: Some("Skin"),
            timestamp_writes: None,
        });
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(1, &*bind_group, &[]);
        pass.dispatch_workgroups(x, y, z);
        let _ = WORKGROUP_SIZE;
    }
}

fn storage_buffer<T>(device: &wgpu::Device, label: &str, elements: usize) -> wgpu::Buffer {
    device.create_buffer(&wgpu::BufferDescriptor {
        label: Some(label),
        size: (elements.max(1) * std::mem::size_of::<T>()) as u64,
        usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    })
}

/// An empty group-0 layout.
///
/// `skinning.wgsl` includes `global_pool.wgsl` for the `Vertex` struct alone
/// and binds nothing in group 0, but WGSL's group numbering is positional in a
/// pipeline layout — group 1 cannot be the first entry.
fn empty_layout(device: &wgpu::Device) -> wgpu::BindGroupLayout {
    device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("Skin empty group 0"),
        entries: &[],
    })
}
