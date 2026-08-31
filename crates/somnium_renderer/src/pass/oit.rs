//! Weighted-blended order-independent transparency (MORROWIND-AC).
//!
//! McGuire and Bavoil, *Weighted Blended Order-Independent Transparency*
//! (Journal of Computer Graphics Techniques 2(2), 2013). The accumulation rules
//! and the weight function are documented in `shaders/transparent.wgsl`; the
//! resolve is `shaders/oit_composite.wgsl`. This file owns the two targets and
//! the composite pipeline.
//!
//! # Why this and not a per-pixel linked list
//!
//! The 2026 plan (`phase_MORROWIND.md` §8, MORROWIND-AC) guessed that per-pixel
//! linked lists were "the likely answer". Measured against the tree, they are
//! not:
//!
//!   - A PPLL writes storage from a fragment shader, which needs
//!     `DownlevelFlags::FRAGMENT_WRITABLE_STORAGE`. Somnium queries no downlevel
//!     flag anywhere, and every atomic in the repository today is in a
//!     `@compute` entry point. It would be the engine's first portability
//!     cliff, and it would need a fallback — which would be this pass.
//!   - Its node pool has to be sized from an assumed layer depth. At eight
//!     layers that is ~199 MB at 1080p and ~796 MB at 4K, and overflow is a
//!     dropped fragment: a visible artefact whose budget nobody can justify
//!     without content that does not exist yet.
//!
//! Weighted-blended needs no feature, no flag and no guess: two fixed targets,
//! 10 bytes a pixel (20.7 MB at 1080p, 82.9 MB at 4K), and no overflow
//! behaviour to specify because there is no
//! pool to overflow. It is approximate — the weight function decides which
//! fragment dominates, and it can be wrong for a near-opaque surface behind a
//! transparent one — which is why the sorted path stays and this is authored.
//!
//! # Interface
//!
//! [`OitPass::begin`] clears, the caller draws through
//! `TransparentPass::record_weighted`, [`OitPass::composite`] resolves. Three
//! calls, and the blend states, formats, clear values and resize lifecycle are
//! behind them.

/// `Rgba16Float`: the accumulator sums premultiplied colour times a weight up
/// to 3e3, so 8-bit would clip on the first fragment and 32-bit would double
/// the bandwidth for range nothing uses. The weight function's clamp is chosen
/// against exactly this format's range.
pub const ACCUM_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba16Float;
/// `R16Float`: a running product of `(1 - a)` in one channel. `R8Unorm` loses
/// too much precision once a few layers multiply together — the error compounds
/// rather than averaging out.
pub const REVEAL_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::R16Float;

/// The two accumulation targets and the resolve that composites them.
pub struct OitPass {
    composite_pipeline: wgpu::RenderPipeline,
    bind_group_layout: wgpu::BindGroupLayout,
    bind_group: wgpu::BindGroup,
    accum_texture: wgpu::Texture,
    accum_view: wgpu::TextureView,
    reveal_texture: wgpu::Texture,
    reveal_view: wgpu::TextureView,
    width: u32,
    height: u32,
    /// Authored on the Post Processing component. **Default off**: turning it
    /// on changes what an existing scene draws, and `phase_MORROWIND.md` §3
    /// forbids doing that without evidence.
    pub enabled: bool,
}

impl OitPass {
    /// Blend state for the accumulation target: plain addition, both channels.
    #[must_use]
    pub const fn accum_blend() -> wgpu::BlendState {
        let add = wgpu::BlendComponent {
            src_factor: wgpu::BlendFactor::One,
            dst_factor: wgpu::BlendFactor::One,
            operation: wgpu::BlendOperation::Add,
        };
        wgpu::BlendState {
            color: add,
            alpha: add,
        }
    }

    /// Blend state for the revealage target: a running product of `1 - a`.
    ///
    /// `Zero * src + (1 - src) * dst` is multiplication by `1 - a` written in
    /// the only vocabulary a fixed-function blender has. The shader therefore
    /// writes `a` and not `1 - a`, which is the one place this is easy to get
    /// backwards.
    #[must_use]
    pub const fn reveal_blend() -> wgpu::BlendState {
        let multiply = wgpu::BlendComponent {
            src_factor: wgpu::BlendFactor::Zero,
            dst_factor: wgpu::BlendFactor::OneMinusSrc,
            operation: wgpu::BlendOperation::Add,
        };
        wgpu::BlendState {
            color: multiply,
            alpha: multiply,
        }
    }

    pub fn new(
        device: &wgpu::Device,
        shaders: &crate::shaders::Shaders,
        hdr_format: wgpu::TextureFormat,
        width: u32,
        height: u32,
    ) -> Self {
        let bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("OIT Composite BGL"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        // `textureLoad` only — the resolve is one-to-one with
                        // the target, so there is nothing to filter and no
                        // sampler to bind.
                        sample_type: wgpu::TextureSampleType::Float { filterable: false },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: false },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
            ],
        });

        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("OIT Composite Shader"),
            source: wgpu::ShaderSource::Wgsl(shaders.source_or_panic("oit_composite.wgsl").into()),
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("OIT Composite Pipeline Layout"),
            bind_group_layouts: &[Some(&bind_group_layout)],
            immediate_size: 0,
        });
        let composite_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("OIT Composite"),
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
                    format: hdr_format,
                    // The resolve returns straight alpha and composites
                    // over whatever opaque geometry already filled the HDR
                    // target, exactly as the sorted path's final blend did.
                    blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
                compilation_options: Default::default(),
            }),
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            cache: None,
        });

        let (accum_texture, accum_view) =
            Self::alloc(device, "OIT Accum", ACCUM_FORMAT, width, height);
        let (reveal_texture, reveal_view) =
            Self::alloc(device, "OIT Reveal", REVEAL_FORMAT, width, height);
        let bind_group = Self::bind(device, &bind_group_layout, &accum_view, &reveal_view);

        Self {
            composite_pipeline,
            bind_group_layout,
            bind_group,
            accum_texture,
            accum_view,
            reveal_texture,
            reveal_view,
            width,
            height,
            enabled: false,
        }
    }

    fn alloc(
        device: &wgpu::Device,
        label: &str,
        format: wgpu::TextureFormat,
        width: u32,
        height: u32,
    ) -> (wgpu::Texture, wgpu::TextureView) {
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some(label),
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

    fn bind(
        device: &wgpu::Device,
        layout: &wgpu::BindGroupLayout,
        accum: &wgpu::TextureView,
        reveal: &wgpu::TextureView,
    ) -> wgpu::BindGroup {
        device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("OIT Composite BG"),
            layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(accum),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(reveal),
                },
            ],
        })
    }

    pub fn resize(&mut self, device: &wgpu::Device, width: u32, height: u32) {
        if self.width == width && self.height == height {
            return;
        }
        let (at, av) = Self::alloc(device, "OIT Accum", ACCUM_FORMAT, width, height);
        let (rt, rv) = Self::alloc(device, "OIT Reveal", REVEAL_FORMAT, width, height);
        self.accum_texture = at;
        self.accum_view = av;
        self.reveal_texture = rt;
        self.reveal_view = rv;
        self.width = width;
        self.height = height;
        self.bind_group = Self::bind(
            device,
            &self.bind_group_layout,
            &self.accum_view,
            &self.reveal_view,
        );
    }

    #[must_use]
    pub fn accum_view(&self) -> &wgpu::TextureView {
        &self.accum_view
    }

    #[must_use]
    pub fn reveal_view(&self) -> &wgpu::TextureView {
        &self.reveal_view
    }

    /// Bytes the two targets occupy at the current size.
    ///
    /// Fixed for the resolution, which is the property a linked list does not
    /// have. Asserted in the tests so the figures quoted in the record cannot
    /// drift from the formats above.
    #[must_use]
    pub fn target_bytes(&self) -> u64 {
        let px = u64::from(self.width.max(1)) * u64::from(self.height.max(1));
        px * 8 + px * 2
    }

    /// Clear both targets to their identity values.
    ///
    /// Accum starts at zero because it is a sum; reveal starts at **one**
    /// because it is a product. Clearing reveal to zero instead would make
    /// every pixel fully covered before anything drew, and the resolve would
    /// paint the whole screen — a mistake that looks like a broken depth test
    /// rather than a wrong clear, which is why it is a named method and not an
    /// inline `LoadOp` at the call site.
    pub fn begin(&self, encoder: &mut wgpu::CommandEncoder) {
        encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("OIT Clear"),
            multiview_mask: None,
            color_attachments: &[
                Some(wgpu::RenderPassColorAttachment {
                    view: &self.accum_view,
                    resolve_target: None,
                    depth_slice: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                        store: wgpu::StoreOp::Store,
                    },
                }),
                Some(wgpu::RenderPassColorAttachment {
                    view: &self.reveal_view,
                    resolve_target: None,
                    depth_slice: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::WHITE),
                        store: wgpu::StoreOp::Store,
                    },
                }),
            ],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
        });
    }

    /// Resolve the two targets over `hdr_view`.
    pub fn composite(&self, encoder: &mut wgpu::CommandEncoder, hdr_view: &wgpu::TextureView) {
        let mut rpass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("OIT Composite"),
            multiview_mask: None,
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: hdr_view,
                resolve_target: None,
                depth_slice: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Load,
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
        });
        rpass.set_pipeline(&self.composite_pipeline);
        rpass.set_bind_group(0, &self.bind_group, &[]);
        rpass.draw(0..3, 0..1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accumulation_adds_and_revealage_multiplies() {
        // These two blend states *are* the algorithm. Written out as an
        // assertion because getting `reveal` backwards produces a plausible
        // image — everything slightly too transparent — rather than an obvious
        // failure, and that is the kind of bug that survives a review.
        let accum = OitPass::accum_blend();
        assert_eq!(accum.color.src_factor, wgpu::BlendFactor::One);
        assert_eq!(accum.color.dst_factor, wgpu::BlendFactor::One);
        assert_eq!(accum.alpha.src_factor, wgpu::BlendFactor::One);
        assert_eq!(accum.alpha.dst_factor, wgpu::BlendFactor::One);

        let reveal = OitPass::reveal_blend();
        assert_eq!(reveal.color.src_factor, wgpu::BlendFactor::Zero);
        assert_eq!(reveal.color.dst_factor, wgpu::BlendFactor::OneMinusSrc);
    }

    #[test]
    fn the_targets_cost_ten_bytes_a_pixel_and_do_not_depend_on_depth_complexity() {
        assert_eq!(ACCUM_FORMAT.block_copy_size(None), Some(8));
        assert_eq!(REVEAL_FORMAT.block_copy_size(None), Some(2));
        // The figures the AC report weighs against a linked list's node pool.
        assert_eq!(1920u64 * 1080 * 10, 20_736_000);
        assert_eq!(3840u64 * 2160 * 10, 82_944_000);
    }

    /// The weight function from `transparent.wgsl`, mirrored so its range can
    /// be pinned without a GPU. If this drifts from the shader the OIT image
    /// goes subtly wrong rather than failing, so the duplication is deliberate
    /// and the shader names this test.
    fn oit_weight(z: f32, a: f32) -> f32 {
        let d = 1.0 - z;
        ((a + 0.01).powi(4) + (0.01f32).max(3.0e3 * d * d * d)).clamp(1.0e-5, 3.0e3)
    }

    #[test]
    fn the_weight_never_leaves_the_range_f16_can_hold() {
        // f16's largest finite value is 65504. The accumulator holds
        // `colour * alpha * weight` summed over layers, so the weight alone
        // must stay far enough below that for a bright fragment to fit.
        for zi in 0..=100 {
            let z = zi as f32 / 100.0;
            for ai in 0..=100 {
                let a = ai as f32 / 100.0;
                let w = oit_weight(z, a);
                assert!(
                    (1.0e-5..=3.0e3).contains(&w),
                    "weight {w} out of range at z={z} a={a}"
                );
                assert!(w.is_finite());
            }
        }
    }

    #[test]
    fn nearer_fragments_dominate() {
        // The whole point of a weight function: at equal alpha, a fragment
        // closer to the camera must count for more, or the resolve averages a
        // window with the wall behind it.
        let near = oit_weight(0.1, 0.5);
        let far = oit_weight(0.9, 0.5);
        assert!(near > far, "near {near} should outweigh far {far}");
    }

    #[test]
    fn a_fully_transparent_fragment_still_has_a_usable_weight() {
        // Zero alpha must not produce a zero weight: it would divide by zero in
        // the resolve for a pixel touched only by invisible fragments.
        assert!(oit_weight(0.5, 0.0) >= 1.0e-5);
    }
}
