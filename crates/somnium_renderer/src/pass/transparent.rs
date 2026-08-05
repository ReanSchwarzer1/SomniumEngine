//! Phase 21: forward pass for alpha-blended materials.
//!
//! The visibility buffer resolves exactly one triangle per pixel, so blended
//! surfaces cannot go through it — before this pass, glTF `alphaMode: BLEND`
//! materials (window glass, light lenses, fake shadow planes) rendered fully
//! opaque, which is what turned imported car glass into solid grey panels.
//!
//! Blended draws are collected into their own queue, sorted back-to-front by
//! distance from the camera, and drawn here after opaque shading, water and
//! terrain have filled the HDR target. Depth is tested against the opaque
//! depth buffer but never written, so blended surfaces occlude nothing.
//!
//! Sorting per-object rather than per-triangle is the usual trade: it is cheap
//! and correct for separated panes, and can still be wrong where two blended
//! surfaces of the same object intersect.

/// A blended draw plus the depth used to sort it.
pub struct TransparentDraw {
    pub instance_index: u32,
    pub index_count: u32,
    /// Squared distance from the camera to the instance origin.
    pub depth_sq: f32,
}

pub struct TransparentPass {
    pipeline: wgpu::RenderPipeline,
    /// Sampler + environment cubemap for reflections.
    bind_group: wgpu::BindGroup,
}

impl TransparentPass {
    pub fn new(
        device: &wgpu::Device,
        target_format: wgpu::TextureFormat,
        global_layout: &wgpu::BindGroupLayout,
        env_view: &wgpu::TextureView,
        env_sampler: &wgpu::Sampler,
    ) -> Self {
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("Transparent BGL"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::Cube,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });

        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("Transparent Sampler"),
            address_mode_u: wgpu::AddressMode::Repeat,
            address_mode_v: wgpu::AddressMode::Repeat,
            address_mode_w: wgpu::AddressMode::Repeat,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::MipmapFilterMode::Linear,
            ..Default::default()
        });

        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("Transparent BG"),
            layout: &layout,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::Sampler(&sampler) },
                wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::TextureView(env_view) },
                wgpu::BindGroupEntry { binding: 2, resource: wgpu::BindingResource::Sampler(env_sampler) },
            ],
        });

        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("Transparent Shader"),
            source: wgpu::ShaderSource::Wgsl(include_str!("../shaders/transparent.wgsl").into()),
        });

        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("Transparent Pipeline Layout"),
            bind_group_layouts: &[Some(global_layout), Some(&layout)],
            immediate_size: 0,
        });

        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("Transparent Pipeline"),
            layout: Some(&pipeline_layout),
            multiview_mask: None,
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                buffers: &[], // programmable vertex pulling
                compilation_options: Default::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_main"),
                targets: &[Some(wgpu::ColorTargetState {
                    format: target_format,
                    blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
                compilation_options: Default::default(),
            }),
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                // Blended geometry is overwhelmingly double-sided in glTF
                // (thin glass), and the shader flips the normal on back faces.
                cull_mode: None,
                ..Default::default()
            },
            depth_stencil: Some(wgpu::DepthStencilState {
                format: wgpu::TextureFormat::Depth32Float,
                // Test against opaque depth, but never write: blended surfaces
                // must not occlude each other or anything drawn later.
                depth_write_enabled: Some(false),
                depth_compare: Some(wgpu::CompareFunction::LessEqual),
                stencil: wgpu::StencilState::default(),
                bias: wgpu::DepthBiasState::default(),
            }),
            multisample: wgpu::MultisampleState::default(),
            cache: None,
        });

        Self { pipeline, bind_group }
    }

    /// Draw the blended queue. `draws` must already be sorted back-to-front.
    pub fn record(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        target_view: &wgpu::TextureView,
        depth_view: &wgpu::TextureView,
        global_bind_group: &wgpu::BindGroup,
        draws: &[TransparentDraw],
    ) {
        if draws.is_empty() {
            return;
        }

        let mut rpass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("Transparent Pass"),
            multiview_mask: None,
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: target_view,
                resolve_target: None,
                depth_slice: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Load,
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                view: depth_view,
                depth_ops: None, // read-only
                stencil_ops: None,
            }),
            timestamp_writes: None,
            occlusion_query_set: None,
        });

        rpass.set_pipeline(&self.pipeline);
        rpass.set_bind_group(0, global_bind_group, &[]);
        rpass.set_bind_group(1, &self.bind_group, &[]);
        for d in draws {
            rpass.draw(0..d.index_count, d.instance_index..d.instance_index + 1);
        }
    }
}

/// Sort blended draws back-to-front (furthest first), which is the order
/// alpha blending requires.
pub fn sort_back_to_front(draws: &mut [TransparentDraw]) {
    draws.sort_by(|a, b| {
        b.depth_sq
            .partial_cmp(&a.depth_sq)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn d(depth_sq: f32) -> TransparentDraw {
        TransparentDraw { instance_index: 0, index_count: 3, depth_sq }
    }

    #[test]
    fn furthest_draws_come_first() {
        let mut v = vec![d(1.0), d(100.0), d(25.0)];
        sort_back_to_front(&mut v);
        let order: Vec<f32> = v.iter().map(|x| x.depth_sq).collect();
        assert_eq!(order, vec![100.0, 25.0, 1.0]);
    }

    #[test]
    fn a_nan_depth_does_not_panic() {
        // partial_cmp returns None for NaN; the comparator must stay total or
        // sort_by panics with "user-provided comparison function does not
        // correctly implement a total order".
        let mut v = vec![d(5.0), d(f32::NAN), d(1.0)];
        sort_back_to_front(&mut v);
        assert_eq!(v.len(), 3);
    }

    #[test]
    fn already_sorted_input_is_left_alone() {
        let mut v = vec![d(9.0), d(4.0), d(1.0)];
        sort_back_to_front(&mut v);
        let order: Vec<f32> = v.iter().map(|x| x.depth_sq).collect();
        assert_eq!(order, vec![9.0, 4.0, 1.0]);
    }
}
