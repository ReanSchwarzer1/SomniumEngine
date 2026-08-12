//! Temporal anti-aliasing (Phase 24F).
//!
//! Owns the history buffers and the jitter sequence. The visibility buffer
//! cannot use MSAA, so before this the only anti-aliasing available was FXAA,
//! which smooths edges it can see in a single frame and does nothing for the
//! sub-pixel sparkle that thin geometry produces.
//!
//! Also the prerequisite for 24H and 24I: those techniques trade samples for
//! noise, and without something accumulating across frames there is nothing to
//! turn that noise back into an image.

/// How much history to keep per frame.
///
/// 0.9 converges in roughly ten frames. Higher is smoother but ghosts longer
/// when the neighbourhood clip fails to catch stale history.
const BLEND_FACTOR: f32 = 0.9;

/// Length of the jitter cycle.
///
/// Eight is enough to resolve edges without the pattern's period becoming
/// visible as a slow wobble on near-static geometry.
const JITTER_PERIOD: u32 = 8;

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct TaaParams {
    inv_view_proj: [[f32; 4]; 4],
    prev_view_proj: [[f32; 4]; 4],
    inv_resolution: [f32; 2],
    blend_factor: f32,
    history_valid: f32,
    debug_mode: u32,
    /// Minimum depth advantage before closest-depth dilation prefers a
    /// neighbour. Guards against near-ties on smooth surfaces flipping the
    /// choice every frame. `SOMNIUM_TAA_DILATE_EPS=0` restores the old
    /// behaviour for A/B comparison.
    dilation_epsilon: f32,
    // 152 bytes of fields; WGSL rounds the struct up to its 16-byte alignment
    // (from the mat4x4s), so this must reach 176 rather than the 164 that
    // `[u32; 3]` gave — wgpu rejected the bind group outright, which is the
    // good outcome for a mismatch this easy to introduce.
    _pad: [u32; 6],
}

pub struct TaaPass {
    pipeline: wgpu::RenderPipeline,
    layout: wgpu::BindGroupLayout,
    sampler: wgpu::Sampler,
    params: wgpu::Buffer,

    /// Ping-pong pair: one holds last frame's result, the other receives this
    /// frame's. A single buffer cannot work — the pass reads history while
    /// writing the new frame, and wgpu forbids binding one texture as both.
    history: [wgpu::Texture; 2],
    history_views: [wgpu::TextureView; 2],
    /// Bind groups indexed by which history buffer is being *read*.
    bind_groups: Option<[wgpu::BindGroup; 2]>,
    write_index: usize,

    prev_view_proj: glam::Mat4,
    frame_index: u32,
    /// Cleared on resize and on the first frame, when no history exists.
    history_valid: bool,
    enabled: bool,
    /// Metered exposure, for normalising the blend space.
    exposure_buffer: wgpu::Buffer,
    /// Debug visualisation selector, from `SOMNIUM_TAA_DEBUG`.
    ///
    /// 1 raw history · 2 clipped history · 3 current · 4 neighbourhood min
    /// 5 neighbourhood max · 6 clip/clamp flags · 7 history-vs-current delta
    /// 8 |prev_uv - uv| in pixels.
    debug_mode: u32,
    /// Minimum depth advantage before dilation prefers a neighbour, from
    /// `SOMNIUM_TAA_DILATE_EPS`. 0 restores the unguarded behaviour.
    dilation_epsilon: f32,
}

impl TaaPass {
    pub fn new(
        device: &wgpu::Device,
        format: wgpu::TextureFormat,
        width: u32,
        height: u32,
        exposure_buffer: &wgpu::Buffer,
    ) -> Self {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("taa.wgsl"),
            source: wgpu::ShaderSource::Wgsl(include_str!("../shaders/taa.wgsl").into()),
        });

        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("TAA BGL"),
            entries: &[
                texture_entry(0, wgpu::TextureSampleType::Float { filterable: true }),
                texture_entry(1, wgpu::TextureSampleType::Float { filterable: true }),
                texture_entry(2, wgpu::TextureSampleType::Depth),
                wgpu::BindGroupLayoutEntry {
                    binding: 3,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 4,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                // The metered exposure, so the blend can work in exposed space.
                wgpu::BindGroupLayoutEntry {
                    binding: 5,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Storage { read_only: true },
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                texture_entry(6, wgpu::TextureSampleType::Float { filterable: false }),
                texture_entry(7, wgpu::TextureSampleType::Float { filterable: false }),
            ],
        });

        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("TAA PL"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });

        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("TAA Pipeline"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                buffers: &[],
                compilation_options: wgpu::PipelineCompilationOptions::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_main"),
                targets: &[Some(wgpu::ColorTargetState {
                    format,
                    blend: None,
                    write_mask: wgpu::ColorWrites::ALL,
                })],
                compilation_options: wgpu::PipelineCompilationOptions::default(),
            }),
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview_mask: None,
            cache: None,
        });

        let (history, history_views) = Self::make_history(device, format, width, height);

        Self {
            pipeline,
            layout,
            sampler: device.create_sampler(&wgpu::SamplerDescriptor {
                label: Some("TAA sampler"),
                address_mode_u: wgpu::AddressMode::ClampToEdge,
                address_mode_v: wgpu::AddressMode::ClampToEdge,
                mag_filter: wgpu::FilterMode::Linear,
                min_filter: wgpu::FilterMode::Linear,
                ..Default::default()
            }),
            params: device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("TAA params"),
                size: std::mem::size_of::<TaaParams>() as u64,
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            }),
            history,
            history_views,
            bind_groups: None,
            write_index: 0,
            prev_view_proj: glam::Mat4::IDENTITY,
            frame_index: 0,
            exposure_buffer: exposure_buffer.clone(),
            history_valid: false,
            enabled: std::env::var("SOMNIUM_TAA").as_deref() != Ok("0"),
            dilation_epsilon: std::env::var("SOMNIUM_TAA_DILATE_EPS")
                .ok()
                .and_then(|v| v.parse::<f32>().ok())
                .unwrap_or(4.0),
            debug_mode: std::env::var("SOMNIUM_TAA_DEBUG")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(0),
        }
    }

    fn make_history(
        device: &wgpu::Device,
        format: wgpu::TextureFormat,
        width: u32,
        height: u32,
    ) -> ([wgpu::Texture; 2], [wgpu::TextureView; 2]) {
        let make = || {
            device.create_texture(&wgpu::TextureDescriptor {
                label: Some("TAA history"),
                size: wgpu::Extent3d {
                    width: width.max(1),
                    height: height.max(1),
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                    | wgpu::TextureUsages::TEXTURE_BINDING
                    | wgpu::TextureUsages::COPY_SRC,
                view_formats: &[],
            })
        };
        let a = make();
        let b = make();
        let va = a.create_view(&wgpu::TextureViewDescriptor::default());
        let vb = b.create_view(&wgpu::TextureViewDescriptor::default());
        ([a, b], [va, vb])
    }

    pub fn set_enabled(&mut self, enabled: bool) {
        if enabled != self.enabled {
            // Coming back on with a stale history would blend in a frame from
            // whenever TAA was last active.
            self.history_valid = false;
        }
        self.enabled = enabled;
    }

    pub fn enabled(&self) -> bool {
        self.enabled
    }

    /// True when a debug view is active, so the caller can bypass tone mapping
    /// and show the raw values rather than a graded version of them.
    pub fn debugging(&self) -> bool {
        self.debug_mode != 0
    }

    /// Sub-pixel offset for this frame's projection, in NDC.
    ///
    /// Returns zero when disabled, so the projection is untouched rather than
    /// jittered into a permanently blurry image with nothing accumulating it.
    pub fn jitter_ndc(&self, width: u32, height: u32) -> glam::Vec2 {
        if !self.enabled || width == 0 || height == 0 {
            return glam::Vec2::ZERO;
        }
        // Halton (2,3) — the sequence most published TAA uses, which makes
        // comparisons against reference implementations meaningful.
        let i = self.frame_index % JITTER_PERIOD;
        let x = halton(i + 1, 2) - 0.5;
        let y = halton(i + 1, 3) - 0.5;
        // One pixel in NDC is 2 / resolution.
        glam::Vec2::new(x * 2.0 / width as f32, y * 2.0 / height as f32)
    }

    pub fn resize(
        &mut self,
        device: &wgpu::Device,
        format: wgpu::TextureFormat,
        width: u32,
        height: u32,
    ) {
        let (history, history_views) = Self::make_history(device, format, width, height);
        self.history = history;
        self.history_views = history_views;
        self.bind_groups = None;
        self.history_valid = false;
    }

    /// Build the bind groups if they are missing.
    ///
    /// Deferred rather than done in `new`, because the depth view belongs to
    /// the visibility pass, which is constructed later.
    pub fn ensure_bind_groups(
        &mut self,
        device: &wgpu::Device,
        current_view: &wgpu::TextureView,
        depth_view: &wgpu::TextureView,
        velocity_view: &wgpu::TextureView,
        water_surface_view: &wgpu::TextureView,
    ) {
        if self.bind_groups.is_none() {
            self.rebuild(
                device,
                current_view,
                depth_view,
                velocity_view,
                water_surface_view,
            );
        }
    }

    /// Rebuild bind groups against the current HDR and depth views.
    pub fn rebuild(
        &mut self,
        device: &wgpu::Device,
        current_view: &wgpu::TextureView,
        depth_view: &wgpu::TextureView,
        velocity_view: &wgpu::TextureView,
        water_surface_view: &wgpu::TextureView,
    ) {
        let make = |read: usize| {
            device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("TAA BG"),
                layout: &self.layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: wgpu::BindingResource::TextureView(current_view),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: wgpu::BindingResource::TextureView(&self.history_views[read]),
                    },
                    wgpu::BindGroupEntry {
                        binding: 2,
                        resource: wgpu::BindingResource::TextureView(depth_view),
                    },
                    wgpu::BindGroupEntry {
                        binding: 3,
                        resource: wgpu::BindingResource::Sampler(&self.sampler),
                    },
                    wgpu::BindGroupEntry {
                        binding: 4,
                        resource: self.params.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 5,
                        resource: self.exposure_buffer.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 6,
                        resource: wgpu::BindingResource::TextureView(velocity_view),
                    },
                    wgpu::BindGroupEntry {
                        binding: 7,
                        resource: wgpu::BindingResource::TextureView(water_surface_view),
                    },
                ],
            })
        };
        self.bind_groups = Some([make(0), make(1)]);
    }

    /// Resolve this frame. Returns the view holding the result, which the
    /// caller should use in place of the raw HDR target.
    /// `view_proj_unjittered` reconstructs this frame and is stored for the
    /// next frame.
    ///
    /// **Both ends of the reprojection are un-jittered.**
    ///
    /// Reconstructing with the *jittered* inverse is geometrically exact — it
    /// recovers the world point that actually landed on this pixel — and that
    /// is precisely the trap. Projecting that point with the previous
    /// un-jittered matrix yields `prev_uv = uv - jitter` for a still camera,
    /// so history is fetched from a location that moves every frame. Measured
    /// with `SOMNIUM_TAA_DEBUG=8`, which reports `|prev_uv - uv|` in pixels:
    /// **51 000 of 51 000 sampled pixels were off, with the camera not moving.**
    /// It should be identity everywhere.
    ///
    /// The un-jittered inverse reconstructs along the un-jittered ray, so the
    /// point projects back onto its own pixel exactly and a still camera
    /// reprojects to identity. The cost is a sub-pixel error where depth is
    /// discontinuous, which is the trade every production TAA makes: velocity
    /// is defined between un-jittered positions so a static scene has zero
    /// velocity.
    ///
    /// This was tried once before and reverted after screen-capture frame
    /// deltas said it was worse. Those deltas were later shown to vary from
    /// 0.776 to 2.018 across three runs of one build — pure noise. Mode 8 is
    /// the measurement that settles it, because it has no run-to-run variance
    /// at all.
    pub fn record(
        &mut self,
        encoder: &mut wgpu::CommandEncoder,
        queue: &wgpu::Queue,
        view_proj_unjittered: glam::Mat4,
        width: u32,
        height: u32,
    ) -> Option<&wgpu::TextureView> {
        if !self.enabled {
            self.prev_view_proj = view_proj_unjittered;
            self.history_valid = false;
            return None;
        }
        if std::env::var("SOMNIUM_TAA_MATDBG").is_ok() && self.frame_index < 4 {
            let d: f32 = view_proj_unjittered
                .to_cols_array()
                .iter()
                .zip(self.prev_view_proj.to_cols_array().iter())
                .map(|(a, b)| (a - b).abs())
                .sum();
            tracing::info!(
                "taa frame {}: |unjittered - prev| = {:e}",
                self.frame_index,
                d
            );
        }
        let bind_groups = self.bind_groups.as_ref()?;

        queue.write_buffer(
            &self.params,
            0,
            bytemuck::bytes_of(&TaaParams {
                inv_view_proj: view_proj_unjittered.inverse().to_cols_array_2d(),
                prev_view_proj: self.prev_view_proj.to_cols_array_2d(),
                inv_resolution: [1.0 / width as f32, 1.0 / height as f32],
                dilation_epsilon: self.dilation_epsilon,
                blend_factor: BLEND_FACTOR,
                history_valid: f32::from(u8::from(self.history_valid)),
                debug_mode: self.debug_mode,
                _pad: [0; 6],
            }),
        );

        // Read the buffer written last frame, write the other.
        let read = 1 - self.write_index;
        {
            let mut rpass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("TAA Resolve"),
                multiview_mask: None,
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &self.history_views[self.write_index],
                    resolve_target: None,
                    depth_slice: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
            });
            rpass.set_pipeline(&self.pipeline);
            rpass.set_bind_group(0, &bind_groups[read], &[]);
            rpass.draw(0..3, 0..1);
        }

        let result = self.write_index;
        self.write_index = read;
        self.prev_view_proj = view_proj_unjittered;
        self.frame_index = self.frame_index.wrapping_add(1);
        self.history_valid = true;

        Some(&self.history_views[result])
    }

    /// Texture holding the most recent resolve, for copying back.
    pub fn resolved_texture(&self, index: usize) -> &wgpu::Texture {
        &self.history[index]
    }

    /// Index of the buffer the last `record` wrote to.
    pub fn last_written(&self) -> usize {
        1 - self.write_index
    }
}

fn texture_entry(binding: u32, sample_type: wgpu::TextureSampleType) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::FRAGMENT,
        ty: wgpu::BindingType::Texture {
            sample_type,
            view_dimension: wgpu::TextureViewDimension::D2,
            multisampled: false,
        },
        count: None,
    }
}

/// `index`-th element of the Halton sequence in `base`.
fn halton(mut index: u32, base: u32) -> f32 {
    let mut result = 0.0;
    let mut f = 1.0 / base as f32;
    while index > 0 {
        result += f * (index % base) as f32;
        index /= base;
        f /= base as f32;
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Halton must stay inside the unit interval and not repeat early, or the
    /// jitter pattern collapses and TAA has nothing extra to accumulate.
    #[test]
    fn halton_covers_the_unit_interval_without_repeating() {
        let mut seen: Vec<f32> = Vec::new();
        for i in 1..=JITTER_PERIOD {
            let v = halton(i, 2);
            assert!((0.0..1.0).contains(&v), "halton({i}, 2) = {v}");
            assert!(
                !seen.iter().any(|s| (s - v).abs() < 1e-6),
                "halton repeated {v} within one period",
            );
            seen.push(v);
        }
    }

    /// The first few base-2 elements are 1/2, 1/4, 3/4 — pinning them catches a
    /// digit-reversal mistake, which is easy to make and produces a sequence
    /// that still *looks* plausible.
    #[test]
    fn halton_base_2_matches_the_known_sequence() {
        for (i, expected) in [(1u32, 0.5), (2, 0.25), (3, 0.75), (4, 0.125)] {
            assert!(
                (halton(i, 2) - expected).abs() < 1e-6,
                "halton({i}, 2) = {}, expected {expected}",
                halton(i, 2),
            );
        }
    }
}
