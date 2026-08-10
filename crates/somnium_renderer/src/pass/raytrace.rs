//! Ray-tracing scene: acceleration structures (Phase 24J).
//!
//! The entry point for everything in 24K–24O. Builds a bottom-level
//! acceleration structure per uploaded mesh and a top-level structure per
//! frame, and exposes the TLAS as a bind group so shaders can trace against it.
//!
//! Why hardware first, rather than the software distance-field path: the wgpu
//! feature gap is genuinely just `EXPERIMENTAL_RAY_QUERY`. The binding arrays
//! and non-uniform indexing that Bevy's Solari also needs are already mandatory
//! here for the bindless resource pool, so nothing structural had to change.
//! The software fallback (24P) remains planned for hardware without it.
//!
//! Degrading is not optional. `EXPERIMENTAL_RAY_QUERY` is exactly what its name
//! says, and effectively Vulkan-only, so every entry point here checks whether
//! the device granted it and does nothing when it did not.

use std::collections::HashMap;

/// Geometry registered for tracing, keyed by its vertex offset in the pool.
///
/// The same key the draw queue and meshlet cache use, so a mesh has one
/// identity across the whole renderer rather than three parallel ones.
struct MeshBlas {
    blas: wgpu::Blas,
    /// Kept so the TLAS build can be skipped when nothing references it.
    index_count: u32,
    /// The geometry this BLAS was sized for, so a rebuild can reissue it
    /// without the caller having to remember (Phase 25B).
    size: wgpu::BlasTriangleGeometrySizeDescriptor,
    vertex_offset: u32,
    index_offset: u32,
}

pub struct RaytracePass {
    /// `None` when the device did not grant ray query.
    tlas: Option<wgpu::Tlas>,
    blas: HashMap<u32, MeshBlas>,
    /// Bottom-level structures whose geometry changed and must be (re)built.
    ///
    /// Phase 25B. `build` used to reissue **every** BLAS every frame, which was
    /// affordable only because the scene held a handful of meshes; terrain adds
    /// 256 chunks of 8 192 triangles each and it stops being affordable
    /// immediately. Bevy's `BlasManager` builds only meshes that were added or
    /// modified and rebuilds the top level alone per frame
    /// (`bevy_solari/src/scene/blas.rs`, `binder.rs`), which is what this is.
    pending_blas: Vec<u32>,
    layout: Option<wgpu::BindGroupLayout>,
    bind_group: Option<wgpu::BindGroup>,
    supported: bool,
    /// Instances submitted this frame, rebuilt each time the scene changes.
    instance_count: u32,
}

/// Upper bound on instances in the top-level structure.
///
/// Matches the visibility buffer's own 1022-draw ceiling closely enough that a
/// scene which fits the raster path also fits the ray-traced one; a mismatch
/// would mean geometry that is drawn but cannot be hit, which is worse than a
/// hard limit because it is invisible until something looks wrong.
const MAX_TLAS_INSTANCES: u32 = 1024;

impl RaytracePass {
    pub fn new(device: &wgpu::Device, supported: bool) -> Self {
        if !supported {
            return Self {
                tlas: None,
                blas: HashMap::new(),
                pending_blas: Vec::new(),
                layout: None,
                bind_group: None,
                supported: false,
                instance_count: 0,
            };
        }

        let tlas = device.create_tlas(&wgpu::CreateTlasDescriptor {
            label: Some("Scene TLAS"),
            max_instances: MAX_TLAS_INSTANCES,
            flags: wgpu::AccelerationStructureFlags::PREFER_FAST_TRACE,
            update_mode: wgpu::AccelerationStructureUpdateMode::Build,
        });

        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("Raytrace BGL"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::FRAGMENT | wgpu::ShaderStages::COMPUTE,
                ty: wgpu::BindingType::AccelerationStructure {
                    vertex_return: false,
                },
                count: None,
            }],
        });

        Self {
            tlas: Some(tlas),
            blas: HashMap::new(),
            pending_blas: Vec::new(),
            layout: Some(layout),
            bind_group: None,
            supported: true,
            instance_count: 0,
        }
    }

    pub fn supported(&self) -> bool {
        self.supported
    }

    pub fn layout(&self) -> Option<&wgpu::BindGroupLayout> {
        self.layout.as_ref()
    }

    pub fn bind_group(&self) -> Option<&wgpu::BindGroup> {
        self.bind_group.as_ref()
    }

    /// Register a mesh, building its bottom-level structure.
    ///
    /// Called once per mesh at upload. A BLAS describes geometry in its own
    /// object space, so it is built once and then referenced by however many
    /// instances place it in the world — which is the whole reason the
    /// two-level split exists.
    pub fn register_mesh(
        &mut self,
        device: &wgpu::Device,
        vertex_offset: u32,
        vertex_count: u32,
        index_offset: u32,
        index_count: u32,
    ) {
        if !self.supported || vertex_count == 0 || index_count == 0 {
            return;
        }
        if self.blas.contains_key(&vertex_offset) {
            return;
        }

        let size = wgpu::BlasTriangleGeometrySizeDescriptor {
            vertex_format: wgpu::VertexFormat::Float32x3,
            vertex_count,
            index_format: Some(wgpu::IndexFormat::Uint32),
            index_count: Some(index_count),
            flags: wgpu::AccelerationStructureGeometryFlags::OPAQUE,
        };

        let blas = device.create_blas(
            &wgpu::CreateBlasDescriptor {
                label: Some("Mesh BLAS"),
                flags: wgpu::AccelerationStructureFlags::PREFER_FAST_TRACE,
                update_mode: wgpu::AccelerationStructureUpdateMode::Build,
            },
            wgpu::BlasGeometrySizeDescriptors::Triangles {
                descriptors: vec![size.clone()],
            },
        );

        self.blas.insert(
            vertex_offset,
            MeshBlas { blas, index_count, size, vertex_offset, index_offset },
        );
        self.pending_blas.push(vertex_offset);
    }

    /// Mark an already-registered BLAS as needing a rebuild (Phase 25B).
    ///
    /// Terrain is the first geometry in the engine whose *contents* change
    /// without its allocation changing: a sculpt stroke rewrites a chunk's
    /// heights in place, keeping the same `vertex_offset`, vertex count and
    /// index range. The BLAS is still correctly sized, so it needs rebuilding
    /// rather than recreating — which is also why the chunk spans are stable
    /// (see the note in `geometry.rs`).
    pub fn mark_geometry_dirty(&mut self, vertex_offset: u32) {
        if self.supported
            && self.blas.contains_key(&vertex_offset)
            && !self.pending_blas.contains(&vertex_offset)
        {
            self.pending_blas.push(vertex_offset);
        }
    }

    /// Build any BLAS whose geometry changed, and a TLAS from `instances`.
    ///
    /// `instances` is `(vertex_offset, model matrix)`, matching what the draw
    /// queue already carries, so the traced scene and the rasterised one cannot
    /// drift apart.
    ///
    /// The bottom level is rebuilt **only for geometry that changed** — see
    /// `pending_blas`. Reissuing every BLAS per frame was affordable with a
    /// handful of meshes and is not with a terrain's worth of chunks.
    /// Build the acceleration structures for this frame.
    ///
    /// `instances` is `(instance_buffer_index, vertex_offset, model)`. The first
    /// field is Phase 24L's requirement: `instance_index` on an intersection is
    /// the TLAS *slot*, which is not the instance-buffer index — instances
    /// without a BLAS are skipped below, so the two drift apart the moment one
    /// mesh is missing. Carrying the real index in custom data makes
    /// `instances[hit.instance_custom_data]` exact, and lets a ray hit resolve
    /// through the same array the visibility buffer uses.
    pub fn build(
        &mut self,
        device: &wgpu::Device,
        encoder: &mut wgpu::CommandEncoder,
        vertex_buffer: &wgpu::Buffer,
        index_buffer: &wgpu::Buffer,
        instances: &[(u32, u32, glam::Mat4)],
    ) {
        if !self.supported {
            return;
        }
        let Some(tlas) = self.tlas.as_mut() else {
            return;
        };

        // ── Bottom level ────────────────────────────────────────────────────
        // A BLAS is described by offsets into the engine's single global
        // vertex and index pools rather than owning any memory of its own, so
        // rebuilding one is just re-reading the range it was registered with.
        let pending = std::mem::take(&mut self.pending_blas);
        let entries: Vec<wgpu::BlasBuildEntry> = pending
            .iter()
            .filter_map(|offset| self.blas.get(offset))
            .map(|mesh| wgpu::BlasBuildEntry {
                blas: &mesh.blas,
                geometry: wgpu::BlasGeometries::TriangleGeometries(vec![
                    wgpu::BlasTriangleGeometry {
                        size: &mesh.size,
                        vertex_buffer,
                        // Positions are the first 12 bytes of the 32-byte
                        // vertex, so the whole interleaved pool can be used
                        // directly — no separate position-only copy.
                        first_vertex: mesh.vertex_offset,
                        vertex_stride: std::mem::size_of::<somnium_asset::Vertex>() as u64,
                        index_buffer: Some(index_buffer),
                        first_index: Some(mesh.index_offset),
                        transform_buffer: None,
                        transform_buffer_offset: None,
                    },
                ]),
            })
            .collect();

        // ── Top level ───────────────────────────────────────────────────────
        // Slots are filled densely, by `count` rather than by the draw's index
        // in the queue. Those were the same number while every draw had a BLAS;
        // Phase 25A-2 put terrain chunks in the draw queue, and they have none
        // until 25B, so the two diverge the moment one is skipped.
        //
        // Writing at the queue index instead left holes and, worse, put live
        // instances beyond `count` — which the clear loop below then treats as
        // leftovers and wipes, or misses entirely on a shorter frame. The
        // symptom was not a missing shadow but an *unstable* one: two runs of
        // the same build had the terrain fully lit and fully shadowed, because
        // whether the helmet survived into the TLAS depended on how many
        // terrain chunks sorted ahead of it.
        let mut count = 0u32;
        for (instance_index, vertex_offset, model) in instances.iter() {
            if count >= MAX_TLAS_INSTANCES {
                break;
            }
            let Some(mesh) = self.blas.get(vertex_offset) else {
                continue;
            };
            if mesh.index_count == 0 {
                continue;
            }

            // Acceleration structures want a 3x4 row-major affine transform;
            // glam is column-major, so this is a transpose of the upper 3x4,
            // not a copy.
            let m = model.to_cols_array();
            let transform: [f32; 12] = [
                m[0], m[4], m[8], m[12],
                m[1], m[5], m[9], m[13],
                m[2], m[6], m[10], m[14],
            ];

            tlas[count as usize] = Some(wgpu::TlasInstance::new(
                &mesh.blas,
                transform,
                // Custom data carries the instance-buffer index, so a hit
                // resolves to geometry and material through the same array the
                // visibility buffer uses — one resolve path, not two.
                *instance_index,
                0xff,
            ));
            count += 1;
        }

        // Clear any slot left over from a busier frame; a stale instance would
        // otherwise keep being traced against long after its object was gone.
        for slot in (count as usize)..(self.instance_count as usize) {
            tlas[slot] = None;
        }
        self.instance_count = count;

        encoder.build_acceleration_structures(entries.iter(), std::iter::once(&*tlas));

        if self.bind_group.is_none() {
            if let Some(layout) = self.layout.as_ref() {
                self.bind_group = Some(device.create_bind_group(&wgpu::BindGroupDescriptor {
                    label: Some("Raytrace BG"),
                    layout,
                    entries: &[wgpu::BindGroupEntry {
                        binding: 0,
                        resource: wgpu::BindingResource::AccelerationStructure(tlas),
                    }],
                }));
            }
        }
    }

    /// The scene's top-level structure, for passes that trace against it.
    pub fn tlas(&self) -> Option<&wgpu::Tlas> {
        self.tlas.as_ref()
    }

    pub fn instance_count(&self) -> u32 {
        self.instance_count
    }
}

/// Ray-traced shadow, as the acceptance test for the structures above.
///
/// See the note at the top of `rt_debug.wgsl`: 24J is otherwise unverifiable,
/// because a correctly built acceleration structure and a silently broken one
/// look identical until something traces against them.
pub struct RtDebugPass {
    pipeline: Option<wgpu::ComputePipeline>,
    layout: Option<wgpu::BindGroupLayout>,
    params: Option<wgpu::Buffer>,
    bind: Option<wgpu::BindGroup>,
    pub enabled: bool,
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct RtParams {
    inv_view_proj: [[f32; 4]; 4],
    sun_direction: [f32; 3],
    ray_bias: f32,
}

impl RtDebugPass {
    pub fn new(device: &wgpu::Device, accel_layout: Option<&wgpu::BindGroupLayout>) -> Self {
        let Some(_) = accel_layout else {
            return Self {
                pipeline: None,
                layout: None,
                params: None,
                bind: None,
                enabled: false,
            };
        };

        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("rt_debug.wgsl"),
            source: wgpu::ShaderSource::Wgsl(include_str!("../shaders/rt_debug.wgsl").into()),
        });

        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("RT debug BGL"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::AccelerationStructure {
                        vertex_return: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Depth,
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::StorageTexture {
                        access: wgpu::StorageTextureAccess::WriteOnly,
                        format: wgpu::TextureFormat::Rgba16Float,
                        view_dimension: wgpu::TextureViewDimension::D2,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 3,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
            ],
        });

        let pl = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("RT debug PL"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });

        let pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("RT debug"),
            layout: Some(&pl),
            module: &shader,
            entry_point: Some("main"),
            compilation_options: wgpu::PipelineCompilationOptions::default(),
            cache: None,
        });

        Self {
            pipeline: Some(pipeline),
            layout: Some(layout),
            params: Some(device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("RT debug params"),
                size: std::mem::size_of::<RtParams>() as u64,
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            })),
            bind: None,
            enabled: std::env::var("SOMNIUM_RT_DEBUG").as_deref() == Ok("1"),
        }
    }

    pub fn invalidate(&mut self) {
        self.bind = None;
    }

    #[allow(clippy::too_many_arguments)]
    pub fn record(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        tlas: &wgpu::Tlas,
        depth_view: &wgpu::TextureView,
        out_view: &wgpu::TextureView,
        view_proj: glam::Mat4,
        sun_direction: glam::Vec3,
        width: u32,
        height: u32,
    ) {
        if !self.enabled {
            return;
        }
        let (Some(pipeline), Some(layout), Some(params)) =
            (self.pipeline.as_ref(), self.layout.as_ref(), self.params.as_ref())
        else {
            return;
        };

        if self.bind.is_none() {
            self.bind = Some(device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("RT debug BG"),
                layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: wgpu::BindingResource::AccelerationStructure(tlas),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: wgpu::BindingResource::TextureView(depth_view),
                    },
                    wgpu::BindGroupEntry {
                        binding: 2,
                        resource: wgpu::BindingResource::TextureView(out_view),
                    },
                    wgpu::BindGroupEntry {
                        binding: 3,
                        resource: params.as_entire_binding(),
                    },
                ],
            }));
        }
        let Some(bind) = self.bind.as_ref() else {
            return;
        };

        queue.write_buffer(
            params,
            0,
            bytemuck::bytes_of(&RtParams {
                inv_view_proj: view_proj.inverse().to_cols_array_2d(),
                sun_direction: sun_direction.normalize_or(glam::Vec3::Y).to_array(),
                // Metres. Large enough to clear the surface the ray starts on;
                // too small and every pixel shadows itself.
                ray_bias: 0.05,
            }),
        );

        let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
            label: Some("RT debug"),
            timestamp_writes: None,
        });
        pass.set_pipeline(pipeline);
        pass.set_bind_group(0, bind, &[]);
        pass.dispatch_workgroups(width.div_ceil(8), height.div_ceil(8), 1);
    }
}
