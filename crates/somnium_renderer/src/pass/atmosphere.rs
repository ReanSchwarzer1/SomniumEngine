//! Atmospheric scattering LUTs (Phase 24C).
//!
//! Builds the two tables Hillaire's model needs — transmittance and multiple
//! scattering — once at startup. Neither depends on the sun or the camera, only
//! on the atmosphere's own composition, so there is nothing to update per frame.
//!
//! The sky itself is evaluated straight into the environment cubemap by the IBL
//! pass, which means one sky feeds the background, the ambient light and the
//! reflections. Before this there were three separate hardcoded gradients that
//! could drift apart, and none of them responded to the sun.

/// Transmittance is smooth in its warped parameterisation, so it needs very
/// little resolution. Matches the sizes Bevy and the paper both use.
const TRANSMITTANCE_SIZE: (u32, u32) = (256, 128);
/// Multiple scattering is smoother still.
const MULTISCATTER_SIZE: (u32, u32) = (32, 32);

const LUT_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba16Float;

pub struct AtmospherePass {
    transmittance_view: wgpu::TextureView,
    multiscatter_view: wgpu::TextureView,
    sampler: wgpu::Sampler,
    built: bool,
    transmittance_pipeline: wgpu::RenderPipeline,
    multiscatter_pipeline: wgpu::RenderPipeline,
    layout: wgpu::BindGroupLayout,
    /// Binds a dummy for the transmittance pass, the real LUT for multiscatter.
    transmittance_bind: wgpu::BindGroup,
    multiscatter_bind: wgpu::BindGroup,
}

impl AtmospherePass {
    pub fn new(device: &wgpu::Device, shaders: &crate::shaders::Shaders) -> Self {
        // MORROWIND-C: composition is declared in `atmosphere_lut.wgsl` and
        // resolved by `somnium_shader`; this site no longer knows the order.
        let source = shaders.source_or_panic("atmosphere_lut.wgsl");
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("atmosphere_lut.wgsl"),
            source: wgpu::ShaderSource::Wgsl(source.into()),
        });

        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("Atmosphere LUT BGL"),
            entries: &[
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
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });

        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("Atmosphere LUT PL"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });

        let make_pipeline = |entry: &str, label: &str| {
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(label),
                layout: Some(&pipeline_layout),
                vertex: wgpu::VertexState {
                    module: &shader,
                    entry_point: Some("vs_lut"),
                    buffers: &[],
                    compilation_options: wgpu::PipelineCompilationOptions::default(),
                },
                fragment: Some(wgpu::FragmentState {
                    module: &shader,
                    entry_point: Some(entry),
                    targets: &[Some(wgpu::ColorTargetState {
                        format: LUT_FORMAT,
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
            })
        };

        let make_texture = |size: (u32, u32), label: &str| {
            device
                .create_texture(&wgpu::TextureDescriptor {
                    label: Some(label),
                    size: wgpu::Extent3d {
                        width: size.0,
                        height: size.1,
                        depth_or_array_layers: 1,
                    },
                    mip_level_count: 1,
                    sample_count: 1,
                    dimension: wgpu::TextureDimension::D2,
                    format: LUT_FORMAT,
                    usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                        | wgpu::TextureUsages::TEXTURE_BINDING,
                    view_formats: &[],
                })
                .create_view(&wgpu::TextureViewDescriptor::default())
        };

        let transmittance_view = make_texture(TRANSMITTANCE_SIZE, "Atmosphere transmittance LUT");
        let multiscatter_view = make_texture(MULTISCATTER_SIZE, "Atmosphere multiscatter LUT");

        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("Atmosphere LUT sampler"),
            // Clamped, because both LUTs are parameterised so their edges are
            // the physical limits — wrapping would fold the horizon onto the
            // zenith.
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            address_mode_w: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });

        let make_bind = |view: &wgpu::TextureView, label: &str| {
            device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some(label),
                layout: &layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: wgpu::BindingResource::TextureView(view),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: wgpu::BindingResource::Sampler(&sampler),
                    },
                ],
            })
        };

        // The transmittance pass does not read anything, but the layout still
        // wants a texture; binding its own output is harmless because the pass
        // never samples it.
        let transmittance_bind = make_bind(&multiscatter_view, "Atmosphere transmittance BG");
        let multiscatter_bind = make_bind(&transmittance_view, "Atmosphere multiscatter BG");

        Self {
            transmittance_view,
            multiscatter_view,
            sampler,
            built: false,
            transmittance_pipeline: make_pipeline("fs_transmittance", "Atmosphere transmittance"),
            multiscatter_pipeline: make_pipeline("fs_multiscatter", "Atmosphere multiscatter"),
            layout,
            transmittance_bind,
            multiscatter_bind,
        }
    }

    pub fn transmittance_view(&self) -> &wgpu::TextureView {
        &self.transmittance_view
    }

    pub fn multiscatter_view(&self) -> &wgpu::TextureView {
        &self.multiscatter_view
    }

    pub fn sampler(&self) -> &wgpu::Sampler {
        &self.sampler
    }

    /// Unused, but kept so callers can build compatible bind groups.
    pub fn layout(&self) -> &wgpu::BindGroupLayout {
        &self.layout
    }

    /// Build both LUTs. Does nothing after the first call — they depend only on
    /// the atmosphere's composition, which does not change at runtime.
    pub fn ensure_built(&mut self, device: &wgpu::Device, queue: &wgpu::Queue) {
        if self.built {
            return;
        }
        self.built = true;

        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("Atmosphere LUTs"),
        });

        // Transmittance first: multiple scattering samples it.
        for (pipeline, bind, view, label) in [
            (
                &self.transmittance_pipeline,
                &self.transmittance_bind,
                &self.transmittance_view,
                "Transmittance LUT",
            ),
            (
                &self.multiscatter_pipeline,
                &self.multiscatter_bind,
                &self.multiscatter_view,
                "Multiscatter LUT",
            ),
        ] {
            let mut rpass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some(label),
                multiview_mask: None,
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view,
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
            rpass.set_pipeline(pipeline);
            rpass.set_bind_group(0, bind, &[]);
            rpass.draw(0..3, 0..1);
            drop(rpass);
        }

        // Submitted separately from the frame so the LUTs are ready before the
        // first environment cubemap is generated from them.
        queue.submit(Some(encoder.finish()));
    }
}
