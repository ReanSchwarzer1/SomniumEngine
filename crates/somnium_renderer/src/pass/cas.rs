//! Contrast Adaptive Sharpening (Phase 24AC).
//!
//! The last thing that touches the frame. Post-processing (or FXAA, when it is
//! running) renders into this pass's LDR target instead of the swapchain, and
//! CAS reads that and writes the swapchain — the same handoff `FxaaPass` uses,
//! and the reason both can be chained without either knowing about the other.
//!
//! Runs on the **tone-mapped** image on purpose. CAS measures how much headroom
//! a neighbourhood has left before sharpening would clip, which only means
//! anything once the signal is in the 0..1 range it will actually be displayed
//! in. On HDR values the headroom test is meaningless and the `saturate` at the
//! end would crush every highlight.
//!
//! See `shaders/cas.wgsl` for the filter and the FidelityFX reference.

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct CasParams {
    sharpness: f32,
    strength: f32,
    _pad: [f32; 2],
}

pub struct CasPass {
    pipeline: wgpu::RenderPipeline,
    bind_group_layout: wgpu::BindGroupLayout,
    bind_group: wgpu::BindGroup,
    params_buffer: wgpu::Buffer,
    /// What the stage before CAS renders into.
    input_texture: wgpu::Texture,
    input_view: wgpu::TextureView,
    /// 0 = gentle (least ringing), 1 = maximum. AMD's own knob.
    pub sharpness: f32,
    /// Blend against the unsharpened image, for taste and for the A/B.
    pub strength: f32,
    pub enabled: bool,
}

impl CasPass {
    pub fn new(
        device: &wgpu::Device,
        shaders: &crate::shaders::Shaders,
        surface_format: wgpu::TextureFormat,
        width: u32,
        height: u32,
    ) -> Self {
        let bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("CAS Bind Group Layout"),
            entries: &[
                // No sampler: every tap is a `textureLoad` at an exact texel.
                // CAS is a 3×3 stencil on the pixel grid, and a filtered fetch
                // would blend neighbours before the filter has decided how much
                // of them it wants.
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
            ],
        });

        let params_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("CAS Params"),
            size: std::mem::size_of::<CasParams>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let (input_texture, input_view) = Self::alloc_target(device, surface_format, width, height);
        let bind_group =
            Self::make_bind_group(device, &bind_group_layout, &input_view, &params_buffer);

        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("CAS Shader"),
            source: wgpu::ShaderSource::Wgsl(shaders.source_or_panic("cas.wgsl").into()),
        });

        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("CAS Pipeline Layout"),
            bind_group_layouts: &[Some(&bind_group_layout)],
            immediate_size: 0,
        });

        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("CAS Pipeline"),
            layout: Some(&pipeline_layout),
            multiview_mask: None,
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                buffers: &[],
                compilation_options: Default::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_main"),
                targets: &[Some(wgpu::ColorTargetState {
                    format: surface_format,
                    blend: Some(wgpu::BlendState::REPLACE),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
                compilation_options: Default::default(),
            }),
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            cache: None,
        });

        Self {
            pipeline,
            bind_group_layout,
            bind_group,
            params_buffer,
            input_texture,
            input_view,
            // AMD's default is 0 — "lower ringing". Half way is a reasonable
            // place to start for a TAA'd image, which has more to recover than
            // a sharp one has to lose.
            sharpness: 0.5,
            strength: 1.0,
            enabled: std::env::var("SOMNIUM_CAS").as_deref() != Ok("0"),
        }
    }

    pub fn active(&self) -> bool {
        self.enabled && self.strength > 0.0
    }

    /// The view the stage before CAS should render into.
    pub fn input_view(&self) -> &wgpu::TextureView {
        &self.input_view
    }

    fn alloc_target(
        device: &wgpu::Device,
        format: wgpu::TextureFormat,
        width: u32,
        height: u32,
    ) -> (wgpu::Texture, wgpu::TextureView) {
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("CAS Input"),
            size: wgpu::Extent3d {
                width: width.max(1),
                height: height.max(1),
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        (texture, view)
    }

    fn make_bind_group(
        device: &wgpu::Device,
        layout: &wgpu::BindGroupLayout,
        view: &wgpu::TextureView,
        params: &wgpu::Buffer,
    ) -> wgpu::BindGroup {
        device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("CAS Bind Group"),
            layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: params.as_entire_binding(),
                },
            ],
        })
    }

    pub fn resize(
        &mut self,
        device: &wgpu::Device,
        format: wgpu::TextureFormat,
        width: u32,
        height: u32,
    ) {
        let (texture, view) = Self::alloc_target(device, format, width, height);
        self.input_texture = texture;
        self.input_view = view;
        self.bind_group = Self::make_bind_group(
            device,
            &self.bind_group_layout,
            &self.input_view,
            &self.params_buffer,
        );
    }

    pub fn record(
        &self,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        target: &wgpu::TextureView,
    ) {
        queue.write_buffer(
            &self.params_buffer,
            0,
            bytemuck::bytes_of(&CasParams {
                sharpness: self.sharpness.clamp(0.0, 1.0),
                strength: self.strength.clamp(0.0, 1.0),
                _pad: [0.0; 2],
            }),
        );

        let mut rpass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("CAS Pass"),
            multiview_mask: None,
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: target,
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
        rpass.set_bind_group(0, &self.bind_group, &[]);
        rpass.draw(0..3, 0..1);
    }
}

#[cfg(test)]
mod tests {
    /// The CPU mirror of the shader's per-pixel amount, for the properties that
    /// are easy to state and impossible to see on a hillside.
    fn cas_weight(mn: f32, mx: f32, sharpness: f32) -> f32 {
        let headroom = (mn.min(1.0 - mx) / mx.max(1.0e-5)).clamp(0.0, 1.0);
        headroom.sqrt() * (-1.0 / (8.0 + (5.0 - 8.0) * sharpness.clamp(0.0, 1.0)))
    }

    #[test]
    fn the_lobe_is_negative() {
        // The kernel is `0 A 0 / A 1 A / 0 A 0` with A negative — that is what
        // makes it a sharpen rather than a blur. A positive weight here would
        // silently turn the whole pass into a box filter.
        assert!(cas_weight(0.4, 0.6, 0.5) < 0.0);
    }

    #[test]
    fn a_neighbourhood_with_no_headroom_is_left_alone() {
        // Spanning the full range: sharpening could only clip, so CAS declines.
        // This is the property a fixed-strength unsharp mask cannot have, and
        // the reason CAS does not halo.
        assert_eq!(cas_weight(0.0, 1.0, 1.0), 0.0);
    }

    #[test]
    fn flat_regions_get_the_most_sharpening() {
        let flat = cas_weight(0.5, 0.5, 0.5).abs();
        let contrasty = cas_weight(0.05, 0.95, 0.5).abs();
        assert!(flat > contrasty, "flat {flat} contrasty {contrasty}");
    }

    #[test]
    fn the_sharpness_knob_spans_the_documented_range() {
        // FidelityFX: `-1 / lerp(8, 5, sharpness)`, so a fully flat
        // neighbourhood (headroom 1) lands exactly on -1/8 and -1/5.
        assert!((cas_weight(0.5, 0.5, 0.0) + 0.125).abs() < 1e-6);
        assert!((cas_weight(0.5, 0.5, 1.0) + 0.200).abs() < 1e-6);
    }

    #[test]
    fn a_black_neighbourhood_does_not_produce_nan() {
        // `1/0` is infinity and `0 * inf` is NaN; one NaN pixel spreads through
        // everything downstream that touches it. The shader's `max(mx, 1e-5)`
        // is what stops it, and this is the case that would have found it.
        let w = cas_weight(0.0, 0.0, 0.5);
        assert!(w.is_finite(), "{w}");
        assert_eq!(w, 0.0);
    }
}
