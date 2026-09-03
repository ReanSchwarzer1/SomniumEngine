//! ReSTIR direct lighting (Phase 24K).
//!
//! Owns the reservoir buffers and the visibility target. Builds on 24J's
//! acceleration structures and, like them, does nothing at all when the device
//! did not grant ray query — the shadow map remains the fallback.

/// Visibility output format.
///
/// A single channel would do today, but 24L needs to write radiance through the
/// same plumbing, and widening a format later means touching every consumer.
const VIS_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba16Float;

/// Matches `Reservoir` in `restir_di.wgsl`: 8 floats, 32 bytes.
const RESERVOIR_BYTES: u64 = 32;

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct RestirParams {
    inv_view_proj: [[f32; 4]; 4],
    sun_direction: [f32; 3],
    sun_angular_radius: f32,
    inv_resolution: [f32; 2],
    frame: u32,
    history_valid: f32,
}

pub struct RestirPass {
    pipeline: Option<wgpu::ComputePipeline>,
    layout: Option<wgpu::BindGroupLayout>,
    params: Option<wgpu::Buffer>,
    /// Ping-pong: one frame's reservoirs feed the next frame's reuse, and a
    /// single buffer cannot be read and written in the same dispatch.
    reservoirs: Vec<wgpu::Buffer>,
    binds: Vec<wgpu::BindGroup>,
    grain_view: wgpu::TextureView,
    grain_enabled: bool,
    vis_view: Option<wgpu::TextureView>,
    vis_tex: Option<wgpu::Texture>,
    /// Set once the pass has written the target, so a disabled pass knows it
    /// still has stale contents to clear.
    vis_dirty: bool,
    write_index: usize,
    frame: u32,
    history_valid: bool,
    supported: bool,
    pub enabled: bool,
}

impl RestirPass {
    pub fn new(
        device: &wgpu::Device,
        shaders: &crate::shaders::Shaders,
        supported: bool,
        grain_view: &wgpu::TextureView,
        width: u32,
        height: u32,
    ) -> Self {
        if !supported {
            // The visibility target is still allocated. wgpu zero-fills a new
            // texture, and alpha 0 is exactly the signal shading reads as "no
            // traced result, use the shadow map" — so the fallback path needs
            // no feature check of its own.
            let mut pass = Self {
                pipeline: None,
                layout: None,
                params: None,
                reservoirs: Vec::new(),
                binds: Vec::new(),
                grain_view: grain_view.clone(),
                grain_enabled: false,
                vis_view: None,
                vis_tex: None,
                vis_dirty: false,
                write_index: 0,
                frame: 0,
                history_valid: false,
                supported: false,
                enabled: false,
            };
            pass.allocate_targets(device, width, height);
            return pass;
        }

        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("restir_di.wgsl"),
            source: wgpu::ShaderSource::Wgsl(shaders.source_or_panic("restir_di.wgsl").into()),
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
            label: Some("ReSTIR BGL"),
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
                        format: VIS_FORMAT,
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
                storage(4, true),
                storage(5, false),
                wgpu::BindGroupLayoutEntry {
                    binding: 6,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: false },
                        view_dimension: wgpu::TextureViewDimension::D2Array,
                        multisampled: false,
                    },
                    count: None,
                },
            ],
        });

        let pl = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("ReSTIR PL"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });

        let pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("ReSTIR DI"),
            layout: Some(&pl),
            module: &shader,
            entry_point: Some("main"),
            compilation_options: wgpu::PipelineCompilationOptions::default(),
            cache: None,
        });

        let mut pass = Self {
            pipeline: Some(pipeline),
            layout: Some(layout),
            params: Some(device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("ReSTIR params"),
                size: std::mem::size_of::<RestirParams>() as u64,
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            })),
            reservoirs: Vec::new(),
            binds: Vec::new(),
            grain_view: grain_view.clone(),
            grain_enabled: crate::pass::grain::enabled_by_default("SOMNIUM_DREAMS_GRAIN"),
            vis_view: None,
            vis_tex: None,
            vis_dirty: false,
            write_index: 0,
            frame: 0,
            history_valid: false,
            supported: true,
            enabled: std::env::var("SOMNIUM_RESTIR").as_deref() == Ok("1"),
        };
        pass.resize(device, width, height);
        pass
    }

    /// Select the shared DREAMS sampling sequence at runtime.
    pub fn set_grain_enabled(&mut self, enabled: bool) {
        if self.grain_enabled != enabled {
            self.history_valid = false;
        }
        self.grain_enabled = enabled;
    }

    pub fn supported(&self) -> bool {
        self.supported
    }

    pub fn active(&self) -> bool {
        self.supported && self.enabled
    }

    /// Per-pixel sun visibility, for the shading pass to use in place of the
    /// shadow map.
    pub fn visibility_view(&self) -> Option<&wgpu::TextureView> {
        self.vis_view.as_ref()
    }

    pub fn resize(&mut self, device: &wgpu::Device, width: u32, height: u32) {
        self.allocate_targets(device, width, height);
    }

    fn allocate_targets(&mut self, device: &wgpu::Device, width: u32, height: u32) {
        let w = width.max(1);
        let h = height.max(1);

        let tex = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("ReSTIR visibility"),
            size: wgpu::Extent3d {
                width: w,
                height: h,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: VIS_FORMAT,
            // COPY_DST so the target can be cleared when the pass is switched
            // off — see `clear_if_inactive`.
            usage: wgpu::TextureUsages::STORAGE_BINDING
                | wgpu::TextureUsages::TEXTURE_BINDING
                | wgpu::TextureUsages::RENDER_ATTACHMENT,
            view_formats: &[],
        });
        self.vis_view = Some(tex.create_view(&wgpu::TextureViewDescriptor::default()));
        self.vis_tex = Some(tex);
        self.vis_dirty = false;

        if !self.supported {
            return;
        }

        let size = u64::from(w) * u64::from(h) * RESERVOIR_BYTES;
        self.reservoirs = (0..2)
            .map(|_| {
                device.create_buffer(&wgpu::BufferDescriptor {
                    label: Some("ReSTIR reservoirs"),
                    size,
                    usage: wgpu::BufferUsages::STORAGE,
                    mapped_at_creation: false,
                })
            })
            .collect();

        self.binds.clear();
        // Reservoirs from a different resolution index differently, so history
        // has to be discarded rather than reinterpreted.
        self.history_valid = false;
    }

    fn ensure_binds(
        &mut self,
        device: &wgpu::Device,
        tlas: &wgpu::Tlas,
        depth: &wgpu::TextureView,
    ) {
        if !self.binds.is_empty() {
            return;
        }
        let (Some(layout), Some(params), Some(vis)) = (
            self.layout.as_ref(),
            self.params.as_ref(),
            self.vis_view.as_ref(),
        ) else {
            return;
        };

        // One bind group per ping-pong phase, indexed by which buffer is being
        // written.
        for write in 0..2usize {
            let read = 1 - write;
            self.binds
                .push(device.create_bind_group(&wgpu::BindGroupDescriptor {
                    label: Some("ReSTIR BG"),
                    layout,
                    entries: &[
                        wgpu::BindGroupEntry {
                            binding: 0,
                            resource: wgpu::BindingResource::AccelerationStructure(tlas),
                        },
                        wgpu::BindGroupEntry {
                            binding: 1,
                            resource: wgpu::BindingResource::TextureView(depth),
                        },
                        wgpu::BindGroupEntry {
                            binding: 2,
                            resource: wgpu::BindingResource::TextureView(vis),
                        },
                        wgpu::BindGroupEntry {
                            binding: 3,
                            resource: params.as_entire_binding(),
                        },
                        wgpu::BindGroupEntry {
                            binding: 4,
                            resource: self.reservoirs[read].as_entire_binding(),
                        },
                        wgpu::BindGroupEntry {
                            binding: 5,
                            resource: self.reservoirs[write].as_entire_binding(),
                        },
                        wgpu::BindGroupEntry {
                            binding: 6,
                            resource: wgpu::BindingResource::TextureView(&self.grain_view),
                        },
                    ],
                }));
        }
    }

    /// Zero the visibility target when the pass is switched off.
    ///
    /// Shading treats alpha > 0.5 as "a traced result exists" and prefers it
    /// over the shadow map. `record` early-returns when the pass is inactive
    /// but the texture keeps whatever it last wrote, so turning the feature off
    /// left every surface pinned to a stale traced visibility of 1.0 — shadow
    /// maps stayed overridden and the scene lost its shadows entirely, with no
    /// way back short of a resize. Alpha 0 is the same signal an unsupported
    /// device produces, so clearing restores the fallback exactly.
    pub fn clear_if_inactive(&mut self, encoder: &mut wgpu::CommandEncoder) {
        if self.active() || !self.vis_dirty {
            return;
        }
        if let Some(view) = self.vis_view.as_ref() {
            // A load-op clear, not `clear_texture`: that needs the optional
            // CLEAR_TEXTURE feature, which this device never requests, so it
            // panicked the instant the toggle was switched off.
            encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("ReSTIR visibility clear"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            self.vis_dirty = false;
            // History indexes a different lighting state now; keeping it would
            // reintroduce the stale result the moment the pass came back.
            self.history_valid = false;
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub fn record(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        tlas: &wgpu::Tlas,
        depth_view: &wgpu::TextureView,
        view_proj: glam::Mat4,
        sun_direction: glam::Vec3,
        sun_angular_radius: f32,
        width: u32,
        height: u32,
    ) {
        if !self.active() {
            return;
        }
        self.ensure_binds(device, tlas, depth_view);
        let (Some(pipeline), Some(params)) = (self.pipeline.as_ref(), self.params.as_ref()) else {
            return;
        };
        if self.binds.len() < 2 {
            return;
        }

        queue.write_buffer(
            params,
            0,
            bytemuck::bytes_of(&RestirParams {
                inv_view_proj: view_proj.inverse().to_cols_array_2d(),
                sun_direction: sun_direction.normalize_or(glam::Vec3::Y).to_array(),
                sun_angular_radius,
                inv_resolution: [1.0 / width as f32, 1.0 / height as f32],
                frame: self.frame,
                // Bit 0 = history, bit 1 = DREAMS grain. Packed into the
                // existing scalar to keep the uniform layout stable.
                history_valid: f32::from(u8::from(self.history_valid))
                    + 2.0 * f32::from(u8::from(self.grain_enabled)),
            }),
        );

        let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
            label: Some("ReSTIR DI"),
            timestamp_writes: None,
        });
        pass.set_pipeline(pipeline);
        pass.set_bind_group(0, &self.binds[self.write_index], &[]);
        pass.dispatch_workgroups(width.div_ceil(8), height.div_ceil(8), 1);
        drop(pass);

        self.vis_dirty = true;
        self.write_index = 1 - self.write_index;
        self.frame = self.frame.wrapping_add(1);
        self.history_valid = true;
    }
}
