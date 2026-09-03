//! Terrain clipmap generate (Phase DF).
//!
//! Fragment pass, recorded before shading. Each dirty rectangle of each ring is
//! one draw into that array layer (color attachments). Group 0 is the global
//! pool; group 1 is params + sampler. Compute storage writes sampled as black
//! on this Vulkan path (Dbg 32).

use crate::terrain::clipmap::{ClipmapGenJob, GpuClipmapGen, MAX_TAKE_JOBS, TerrainClipmap};

/// The stable Phase-DF dispatch followed by MORROWIND-AD bindless resources.
/// Keeping these out of `GpuTerrainMaterial` preserves its 2032-byte ABI.
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct GpuClipmapGenVirtual {
    base: GpuClipmapGen,
    /// albedo atlas, surface atlas, page table, atlas edge (texels).
    virtual_texture: [i32; 4],
}

/// wgpu uniform offset alignment. One slot per dirty rectangle.
const PARAMS_STRIDE: u64 = 256;
/// Slots one terrain can consume in a frame: `MAX_TAKE_JOBS` per stack, and
/// [`TerrainClipmapPass::record`] is called once for each of the two.
const SLOTS_PER_TERRAIN: usize = MAX_TAKE_JOBS * 2;

/// Uniform slots handed out within one frame.
///
/// `Queue::write_buffer` does not run at its call site. wgpu applies every
/// pending write just before the frame's command buffers, **in call order**,
/// so two `record` calls that both wrote at offset 0 did not take turns: the
/// second overwrote the first, and then *both* render passes read the second
/// call's uniforms.
///
/// That is a detail ring generated with the macro stack's `rect_min`/`rect_max`,
/// `center`, `origin_uv` and `texels_per_m`. The two rectangles do not
/// coincide, so most of the scissored texels failed `clipmap_generate`'s own
/// bounds test and took its early-out: `albedo = 0`, `surface = (0.5, 0.5,
/// 0.8, 1.0)`. Alpha there is 1.0, which `clipmap_tap_detail` reads as *data*
/// rather than as an ungenerated texel, so the ring keeps a black, flat,
/// hard-edged rectangle that nothing re-dirties and that debug view 34 reports
/// as a healthy detail-ring hit.
///
/// A cursor rather than one buffer per stack, because the collision is between
/// *calls*: a second terrain would alias the first one the same way.
#[derive(Default)]
struct FrameSlots {
    next: usize,
    capacity: usize,
}

impl FrameSlots {
    fn begin(&mut self, capacity: usize) {
        self.next = 0;
        self.capacity = capacity;
    }

    /// Base slot for `count` consecutive jobs, or `None` if the frame is full.
    fn take(&mut self, count: usize) -> Option<usize> {
        let base = self.next;
        let end = base.checked_add(count)?;
        if end > self.capacity {
            return None;
        }
        self.next = end;
        Some(base)
    }
}

pub struct TerrainClipmapPass {
    pipeline: wgpu::RenderPipeline,
    layout: wgpu::BindGroupLayout,
    params: wgpu::Buffer,
    sampler: wgpu::Sampler,
    bind: wgpu::BindGroup,
    /// Slots the params buffer currently holds.
    slots: usize,
    frame: FrameSlots,
}

impl TerrainClipmapPass {
    pub fn new(
        device: &wgpu::Device,
        shaders: &crate::shaders::Shaders,
        global_layout: &wgpu::BindGroupLayout,
    ) -> Self {
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("Terrain clipmap gen BGL"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: true,
                        min_binding_size: std::num::NonZeroU64::new(std::mem::size_of::<
                            GpuClipmapGenVirtual,
                        >()
                            as u64),
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });

        // MORROWIND-C: composition is declared in `clipmap_gen.wgsl` and
        // resolved by `somnium_shader`; this site no longer knows the order.
        let source = shaders.source_or_panic("clipmap_gen.wgsl");
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("Terrain clipmap generate"),
            source: wgpu::ShaderSource::Wgsl(source.into()),
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("Terrain clipmap generate layout"),
            bind_group_layouts: &[Some(global_layout), Some(&layout)],
            immediate_size: 0,
        });
        let target = Some(wgpu::ColorTargetState {
            format: wgpu::TextureFormat::Rgba8Unorm,
            blend: Some(wgpu::BlendState::REPLACE),
            write_mask: wgpu::ColorWrites::ALL,
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("Terrain clipmap generate"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("clipmap_vs"),
                buffers: &[],
                compilation_options: Default::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("clipmap_generate"),
                targets: &[target.clone(), target],
                compilation_options: Default::default(),
            }),
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                strip_index_format: None,
                front_face: wgpu::FrontFace::Ccw,
                cull_mode: None,
                polygon_mode: wgpu::PolygonMode::Fill,
                unclipped_depth: false,
                conservative: false,
            },
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview_mask: None,
            cache: None,
        });

        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("Terrain clipmap generate sampler"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::MipmapFilterMode::Linear,
            address_mode_u: wgpu::AddressMode::Repeat,
            address_mode_v: wgpu::AddressMode::Repeat,
            address_mode_w: wgpu::AddressMode::Repeat,
            ..Default::default()
        });

        let (params, bind) = Self::params_buffer(device, &layout, &sampler, SLOTS_PER_TERRAIN);

        Self {
            pipeline,
            layout,
            params,
            sampler,
            bind,
            slots: SLOTS_PER_TERRAIN,
            frame: FrameSlots::default(),
        }
    }

    fn params_buffer(
        device: &wgpu::Device,
        layout: &wgpu::BindGroupLayout,
        sampler: &wgpu::Sampler,
        slots: usize,
    ) -> (wgpu::Buffer, wgpu::BindGroup) {
        let params = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Terrain clipmap gen params"),
            size: PARAMS_STRIDE * slots as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("Terrain clipmap gen"),
            layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                        buffer: &params,
                        offset: 0,
                        size: std::num::NonZeroU64::new(
                            std::mem::size_of::<GpuClipmapGenVirtual>() as u64,
                        ),
                    }),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(sampler),
                },
            ],
        });
        (params, bind)
    }

    /// Reset the uniform cursor and size the buffer for this frame's terrains.
    ///
    /// Called once, before any `record`, so growing the buffer cannot discard
    /// params an earlier call in the same frame has already written.
    pub fn begin_frame(&mut self, device: &wgpu::Device, terrains: usize) {
        let want = terrains.max(1) * SLOTS_PER_TERRAIN;
        if want > self.slots {
            let (params, bind) = Self::params_buffer(device, &self.layout, &self.sampler, want);
            self.params = params;
            self.bind = bind;
            self.slots = want;
        }
        self.frame.begin(self.slots);
    }

    pub fn record(
        &mut self,
        _device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        global: &wgpu::BindGroup,
        clipmap: &TerrainClipmap,
        terrain_index: u32,
        jobs: &[ClipmapGenJob],
        is_detail: bool,
        virtual_texture: [i32; 4],
    ) {
        if jobs.is_empty() {
            return;
        }
        let n = jobs.len();
        debug_assert!(
            n <= MAX_TAKE_JOBS,
            "take_jobs is the only source of these and caps at {MAX_TAKE_JOBS}"
        );
        // Dropping jobs here would be worse than it reads: `take_jobs` has
        // already cleared their dirty rectangles, so the texels stay marked
        // clean and nothing generates them again.
        let Some(base) = self.frame.take(n) else {
            tracing::error!(
                jobs = n,
                slots = self.slots,
                "clipmap generate ran out of uniform slots; begin_frame must see every terrain"
            );
            return;
        };
        let mut bytes = vec![0u8; PARAMS_STRIDE as usize * n];
        for (i, job) in jobs.iter().enumerate() {
            let params = GpuClipmapGenVirtual {
                base: GpuClipmapGen::from_job(terrain_index, job),
                virtual_texture,
            };
            let start = i * PARAMS_STRIDE as usize;
            bytes[start..start + std::mem::size_of::<GpuClipmapGenVirtual>()]
                .copy_from_slice(bytemuck::bytes_of(&params));
        }
        queue.write_buffer(&self.params, base as u64 * PARAMS_STRIDE, &bytes);

        // One render pass per **ring**, not per rectangle.
        //
        // `take_jobs` walks the generate order ring by ring, so every job for a
        // ring arrives consecutively and they all target the same pair of array
        // layers. Opening a render pass per rectangle meant a begin/end — and
        // the barrier either side of it — for each arm of an L-shaped slide,
        // which is up to four per ring and up to 32 per frame, to write a few
        // thousand texels. Grouping the run costs nothing and issues one draw
        // per rectangle inside a single pass.
        let load = wgpu::Operations {
            load: wgpu::LoadOp::Load,
            store: wgpu::StoreOp::Store,
        };
        let mut start = 0usize;
        while start < n {
            let ring = jobs[start].ring;
            let mut end = start + 1;
            while end < n && jobs[end].ring == ring {
                end += 1;
            }
            let run = start..end;
            start = end;

            if jobs[run.clone()].iter().all(|job| job.rect.is_empty()) {
                continue;
            }
            let (albedo, surface) = if is_detail {
                clipmap.detail_layer(ring)
            } else {
                clipmap.macro_layer(ring)
            };
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("Terrain clipmap generate"),
                color_attachments: &[
                    Some(wgpu::RenderPassColorAttachment {
                        view: albedo,
                        resolve_target: None,
                        depth_slice: None,
                        ops: load,
                    }),
                    Some(wgpu::RenderPassColorAttachment {
                        view: surface,
                        resolve_target: None,
                        depth_slice: None,
                        ops: load,
                    }),
                ],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pass.set_pipeline(&self.pipeline);
            pass.set_bind_group(0, global, &[]);
            for i in run {
                let job = &jobs[i];
                if job.rect.is_empty() {
                    continue;
                }
                pass.set_bind_group(1, &self.bind, &[(base + i) as u32 * PARAMS_STRIDE as u32]);
                pass.set_viewport(
                    job.rect.x as f32,
                    job.rect.y as f32,
                    job.rect.w as f32,
                    job.rect.h as f32,
                    0.0,
                    1.0,
                );
                pass.set_scissor_rect(job.rect.x, job.rect.y, job.rect.w, job.rect.h);
                pass.draw(0..3, 0..1);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Two `record` calls in one frame must not name the same uniform slot.
    ///
    /// The defect this replaces was not a wrong number anywhere: both calls
    /// computed a correct offset into their *own* upload, and both uploads
    /// started at zero. wgpu applies them in call order just before the
    /// frame's passes, so the detail stack was generated with the macro
    /// stack's rectangle, centre and texel density.
    #[test]
    fn one_frame_never_hands_out_a_slot_twice() {
        let mut slots = FrameSlots::default();
        slots.begin(SLOTS_PER_TERRAIN);
        let detail = slots.take(7).expect("room for the detail stack");
        let macro_stack = slots.take(4).expect("room for the macro stack");
        assert_eq!(detail, 0);
        assert_eq!(
            macro_stack, 7,
            "the macro stack starts where the detail stack ended"
        );
    }

    /// A frame's worst case is both stacks, full, for one terrain.
    #[test]
    fn a_frames_capacity_covers_both_stacks_at_their_cap() {
        let mut slots = FrameSlots::default();
        slots.begin(SLOTS_PER_TERRAIN);
        assert!(slots.take(MAX_TAKE_JOBS).is_some(), "detail at its cap");
        assert!(slots.take(MAX_TAKE_JOBS).is_some(), "macro at its cap");
        assert!(slots.take(1).is_none(), "and nothing beyond it");
    }

    #[test]
    fn begin_resets_the_cursor_between_frames() {
        let mut slots = FrameSlots::default();
        slots.begin(SLOTS_PER_TERRAIN);
        assert_eq!(slots.take(5), Some(0));
        slots.begin(SLOTS_PER_TERRAIN);
        assert_eq!(slots.take(5), Some(0), "a new frame starts at zero again");
    }
}
