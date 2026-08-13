//! ReSTIR GI — ray-traced indirect diffuse (Phase 24L).
//!
//! Two compute dispatches per frame over `restir_gi.wgsl`: the first draws one
//! bounce per pixel and merges it with that pixel's history, the second borrows
//! from spatial neighbours, traces the survivor's visibility ray and writes the
//! indirect radiance the shading pass consumes.
//!
//! Like 24J and 24K it does nothing at all when the device did not grant ray
//! query — the constant ambient term remains the fallback, and an output alpha
//! of zero is the signal that says so. wgpu zero-fills a new texture, so the
//! unsupported path needs no feature check of its own.
//!
//! # Bind groups
//!
//! Group 0 is the **shared global pool** — the same bind group the shading pass
//! uses. That is the point of Phase 24L's `global_pool.wgsl` extraction: a ray
//! hit resolves to geometry and material through the same `instances` array a
//! visibility-buffer hit does, so the traced and rasterised views of the scene
//! cannot drift apart. Group 1 is this pass's own.

/// Indirect radiance output. Alpha is the "a traced result exists" flag, on the
/// same convention 24K established for its visibility target.
const GI_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba16Float;

/// Matches `GiReservoir` in `restir_gi.wgsl`: 12 floats, 48 bytes.
const RESERVOIR_BYTES: u64 = 48;

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct GiParams {
    inv_view_proj: [[f32; 4]; 4],
    camera_pos: [f32; 3],
    frame: u32,
    inv_resolution: [f32; 2],
    history_valid: f32,
    intensity: f32,
    max_distance: f32,
    /// Pads to 112, which is what the WGSL mirror rounds to. See the shader's
    /// note on why the three scalars there are not one `vec3`.
    _pad: [f32; 3],
}

#[cfg(test)]
mod tests {
    use super::GiParams;

    /// The mismatch that stopped this pass dispatching the first time it ran.
    ///
    /// wgpu rejected the dispatch outright — "bound with size 112 where the
    /// shader expects 128" — because a trailing `vec3<f32>` aligns to 16 in
    /// WGSL and to 4 in Rust. `tests/shaders_validate.rs` proves the WGSL side
    /// of every struct it checks; this pins the Rust side of this one.
    #[test]
    fn the_params_struct_is_the_112_byte_uniform_layout() {
        assert_eq!(std::mem::size_of::<GiParams>(), 112);
        assert_eq!(std::mem::size_of::<GiParams>() % 16, 0);
    }
}

pub struct RestirGiPass {
    initial: Option<wgpu::ComputePipeline>,
    spatial: Option<wgpu::ComputePipeline>,
    layout: Option<wgpu::BindGroupLayout>,
    params: Option<wgpu::Buffer>,
    sampler: wgpu::Sampler,
    /// `gi_a` carries the previous frame's finished reservoirs; `gi_b` is the
    /// handoff between the two dispatches. Fixed roles, not a ping-pong — see
    /// the shader's comment on why reading and writing one buffer in the
    /// spatial pass would both race and double-count.
    gi_a: Option<wgpu::Buffer>,
    gi_b: Option<wgpu::Buffer>,
    bind: Option<wgpu::BindGroup>,
    out_view: Option<wgpu::TextureView>,
    out_tex: Option<wgpu::Texture>,
    /// Set once the pass has written the target, so a disabled pass knows it
    /// still has stale contents to clear. Same trap 24K hit: switching the
    /// feature off left every surface pinned to the last traced result.
    dirty: bool,
    frame: u32,
    history_valid: bool,
    /// Lighting represented by the temporal reservoirs. A materially changed
    /// sun must discard history so daytime bounce cannot bleed into night.
    last_light: Option<([f32; 3], [f32; 3])>,
    supported: bool,
    pub enabled: bool,
    /// Scales the indirect contribution. Exposed so the A/B can vary the
    /// strength without disabling the pass.
    pub intensity: f32,
    /// Metres beyond which a bounce ray is not traced.
    pub max_distance: f32,
}

impl RestirGiPass {
    pub fn new(
        device: &wgpu::Device,
        global_layout: &wgpu::BindGroupLayout,
        supported: bool,
        width: u32,
        height: u32,
    ) -> Self {
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("ReSTIR GI sampler"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::MipmapFilterMode::Linear,
            ..Default::default()
        });

        let mut pass = Self {
            initial: None,
            spatial: None,
            layout: None,
            params: None,
            sampler,
            gi_a: None,
            gi_b: None,
            bind: None,
            out_view: None,
            out_tex: None,
            dirty: false,
            frame: 0,
            history_valid: false,
            last_light: None,
            supported,
            // On wherever the hardware allows it. `SOMNIUM_RESTIR_GI=0` is the
            // A/B against the environment map's constant diffuse, and a device
            // without ray query never reaches this branch at all.
            enabled: supported && std::env::var("SOMNIUM_RESTIR_GI").as_deref() != Ok("0"),
            intensity: 1.0,
            max_distance: 200.0,
        };

        if !supported {
            pass.allocate_targets(device, width, height);
            return pass;
        }

        // GI first: `enable wgpu_ray_query;` is a directive, and directives must
        // precede every declaration in the module. Declarations themselves are
        // order-independent, which is what lets the pool it depends on be
        // concatenated after it. `tests/shaders_validate.rs` pins this exact
        // concatenation.
        let source = format!(
            "{}\n{}\n{}\n{}\n{}\n{}\n{}\n{}",
            include_str!("../shaders/restir_gi.wgsl"),
            include_str!("../shaders/rt_hit.wgsl"),
            include_str!("../shaders/global_pool.wgsl"),
            include_str!("../shaders/brdf.wgsl"),
            include_str!("../shaders/sampling.wgsl"),
            include_str!("../shaders/atmosphere.wgsl"),
            include_str!("../shaders/hextile.wgsl"),
            include_str!("../shaders/terrain_material.wgsl"),
        );
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("restir_gi.wgsl"),
            source: wgpu::ShaderSource::Wgsl(source.into()),
        });

        let storage = |binding: u32| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::COMPUTE,
            ty: wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Storage { read_only: false },
                has_dynamic_offset: false,
                min_binding_size: None,
            },
            count: None,
        };

        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("ReSTIR GI BGL"),
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
                // The visibility buffer, for the primary surface's geometric
                // normal. A compute shader has no quad to take derivatives
                // across, and a normal reconstructed from neighbouring depths is
                // wrong exactly at the silhouettes indirect light lives on.
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Uint,
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 3,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::StorageTexture {
                        access: wgpu::StorageTextureAccess::WriteOnly,
                        format: GI_FORMAT,
                        view_dimension: wgpu::TextureViewDimension::D2,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 4,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                storage(5),
                storage(6),
                wgpu::BindGroupLayoutEntry {
                    binding: 7,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });

        let pl = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("ReSTIR GI PL"),
            bind_group_layouts: &[Some(global_layout), Some(&layout)],
            immediate_size: 0,
        });

        let make = |entry: &str, label: &str| {
            device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                label: Some(label),
                layout: Some(&pl),
                module: &shader,
                entry_point: Some(entry),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                cache: None,
            })
        };
        pass.initial = Some(make("initial_and_temporal", "ReSTIR GI initial+temporal"));
        pass.spatial = Some(make("spatial_and_shade", "ReSTIR GI spatial+shade"));
        pass.layout = Some(layout);
        pass.params = Some(device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("ReSTIR GI params"),
            size: std::mem::size_of::<GiParams>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        }));
        pass.resize(device, width, height);
        pass
    }

    pub fn supported(&self) -> bool {
        self.supported
    }

    pub fn active(&self) -> bool {
        self.supported && self.enabled
    }

    /// Indirect radiance, for the shading pass to use in place of the constant
    /// ambient term.
    pub fn radiance_view(&self) -> Option<&wgpu::TextureView> {
        self.out_view.as_ref()
    }

    pub fn resize(&mut self, device: &wgpu::Device, width: u32, height: u32) {
        self.allocate_targets(device, width, height);
    }

    fn allocate_targets(&mut self, device: &wgpu::Device, width: u32, height: u32) {
        let w = width.max(1);
        let h = height.max(1);

        let tex = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("ReSTIR GI radiance"),
            size: wgpu::Extent3d {
                width: w,
                height: h,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: GI_FORMAT,
            usage: wgpu::TextureUsages::STORAGE_BINDING
                | wgpu::TextureUsages::TEXTURE_BINDING
                | wgpu::TextureUsages::RENDER_ATTACHMENT,
            view_formats: &[],
        });
        self.out_view = Some(tex.create_view(&wgpu::TextureViewDescriptor::default()));
        self.out_tex = Some(tex);
        self.dirty = false;

        if !self.supported {
            return;
        }

        let size = u64::from(w) * u64::from(h) * RESERVOIR_BYTES;
        let make = |label: &str| {
            device.create_buffer(&wgpu::BufferDescriptor {
                label: Some(label),
                size,
                usage: wgpu::BufferUsages::STORAGE,
                mapped_at_creation: false,
            })
        };
        self.gi_a = Some(make("ReSTIR GI reservoirs A"));
        self.gi_b = Some(make("ReSTIR GI reservoirs B"));
        self.bind = None;
        // Reservoirs from a different resolution index differently, so history
        // has to be discarded rather than reinterpreted.
        self.history_valid = false;
    }

    fn ensure_bind(
        &mut self,
        device: &wgpu::Device,
        tlas: &wgpu::Tlas,
        depth: &wgpu::TextureView,
        vis: &wgpu::TextureView,
    ) {
        if self.bind.is_some() {
            return;
        }
        let (Some(layout), Some(params), Some(out), Some(a), Some(b)) = (
            self.layout.as_ref(),
            self.params.as_ref(),
            self.out_view.as_ref(),
            self.gi_a.as_ref(),
            self.gi_b.as_ref(),
        ) else {
            return;
        };
        self.bind = Some(device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("ReSTIR GI BG"),
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
                    resource: wgpu::BindingResource::TextureView(out),
                },
                wgpu::BindGroupEntry {
                    binding: 4,
                    resource: params.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 5,
                    resource: a.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 6,
                    resource: b.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 7,
                    resource: wgpu::BindingResource::Sampler(&self.sampler),
                },
            ],
        }));
    }

    /// Zero the radiance target when the pass is switched off.
    ///
    /// The same failure 24K documented: `record` early-returns when inactive but
    /// the texture keeps whatever it last wrote, so shading would go on adding a
    /// frozen bounce for ever. Alpha 0 is the signal an unsupported device
    /// produces, so clearing restores the ambient fallback exactly.
    pub fn clear_if_inactive(&mut self, encoder: &mut wgpu::CommandEncoder) {
        if self.active() || !self.dirty {
            return;
        }
        if let Some(view) = self.out_view.as_ref() {
            // A load-op clear, not `clear_texture`: that needs the optional
            // CLEAR_TEXTURE feature this device never requests.
            encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("ReSTIR GI clear"),
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
            self.dirty = false;
            self.history_valid = false;
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub fn record(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        global_bind: &wgpu::BindGroup,
        tlas: &wgpu::Tlas,
        depth_view: &wgpu::TextureView,
        vis_view: &wgpu::TextureView,
        view_proj: glam::Mat4,
        camera_pos: glam::Vec3,
        light_direction: glam::Vec3,
        light_color: glam::Vec3,
        width: u32,
        height: u32,
    ) {
        if !self.active() {
            return;
        }
        let light_key = (light_direction.to_array(), light_color.to_array());
        let light_changed = self.last_light.is_none_or(|(old_direction, old_color)| {
            const MAX_ANGLE_RADIANS: f32 = 0.25_f32.to_radians();
            const MAX_RELATIVE_COLOR_CHANGE: f32 = 0.02;

            let old_direction = glam::Vec3::from_array(old_direction).normalize_or_zero();
            let direction = light_direction.normalize_or_zero();
            let direction_changed = old_direction.dot(direction) < MAX_ANGLE_RADIANS.cos();

            let old_color = glam::Vec3::from_array(old_color);
            let color_scale = old_color
                .abs()
                .max(light_color.abs())
                .max_element()
                .max(1.0);
            let color_changed = (old_color - light_color).abs().max_element()
                > color_scale * MAX_RELATIVE_COLOR_CHANGE;

            direction_changed || color_changed
        });
        if light_changed {
            self.history_valid = false;
            self.last_light = Some(light_key);
        }
        self.ensure_bind(device, tlas, depth_view, vis_view);
        let (Some(initial), Some(spatial), Some(params), Some(bind)) = (
            self.initial.as_ref(),
            self.spatial.as_ref(),
            self.params.as_ref(),
            self.bind.as_ref(),
        ) else {
            return;
        };

        queue.write_buffer(
            params,
            0,
            bytemuck::bytes_of(&GiParams {
                inv_view_proj: view_proj.inverse().to_cols_array_2d(),
                camera_pos: camera_pos.to_array(),
                frame: self.frame,
                inv_resolution: [1.0 / width as f32, 1.0 / height as f32],
                history_valid: f32::from(u8::from(self.history_valid)),
                intensity: self.intensity,
                max_distance: self.max_distance,
                _pad: [0.0; 3],
            }),
        );

        let groups = (width.div_ceil(8), height.div_ceil(8));

        // Two dispatches, not one pass with a barrier between: the spatial pass
        // reads neighbouring pixels' reservoirs, so every pixel of the first
        // dispatch has to have landed before any pixel of the second reads it.
        // Separate dispatches in the same encoder give exactly that ordering.
        {
            let mut cpass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("ReSTIR GI initial+temporal"),
                timestamp_writes: None,
            });
            cpass.set_pipeline(initial);
            cpass.set_bind_group(0, global_bind, &[]);
            cpass.set_bind_group(1, bind, &[]);
            cpass.dispatch_workgroups(groups.0, groups.1, 1);
        }
        {
            let mut cpass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("ReSTIR GI spatial+shade"),
                timestamp_writes: None,
            });
            cpass.set_pipeline(spatial);
            cpass.set_bind_group(0, global_bind, &[]);
            cpass.set_bind_group(1, bind, &[]);
            cpass.dispatch_workgroups(groups.0, groups.1, 1);
        }

        self.dirty = true;
        self.frame = self.frame.wrapping_add(1);
        self.history_valid = true;
    }
}
