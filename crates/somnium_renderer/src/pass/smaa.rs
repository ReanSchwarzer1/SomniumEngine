//! SMAA 1x (MORROWIND-AC).
//!
//! The algorithm and its honest limits are documented at the top of
//! `shaders/smaa.wgsl`; this file owns the GPU resources around it.
//!
//! ## Where it sits in the frame
//!
//! ```text
//! HDR → [PostProcess: tone map] → LDR intermediate
//!                                    ↓ [edges]   → Rg8 edge target
//!                                    ↓ [weights] → Rgba8 weight target
//!                                    ↓ [blend]   → swapchain (or CAS input)
//!                                                     ↓
//!                        gizmos / outline / UI (never blended)
//! ```
//!
//! The same slot FXAA occupies and for the same reason: it needs a tone-mapped
//! image, and it must run before editor chrome so text and gizmo lines stay
//! sharp.
//!
//! ## Interface
//!
//! One `set_mode` in, one `record` out. Three pipelines, two intermediate
//! targets, their resize lifecycle, and the clear discipline between the passes
//! are all behind that.
//!
//! `set_mode` takes resolved scalars rather than `somnium_core`'s
//! `AntiAliasing` enum for a dependency reason worth stating: `somnium_core`
//! depends on this crate, not the other way round, so the authored enum cannot
//! live here — and its `ReflectField` impl could not be written here anyway,
//! since both the trait and the type would be foreign. The authored value stays
//! single upstream (`PostProcessComponent::aa`), and `somnium_core::app` is the
//! adapter that resolves it. That keeps the defect this sub-phase exists to fix
//! from coming back: there is still exactly one thing a user sets.

/// Uniform matching `SmaaParams` in `smaa.wgsl` (16 bytes).
#[repr(C)]
#[derive(Copy, Clone, bytemuck::Pod, bytemuck::Zeroable)]
struct SmaaParams {
    inv_size: [f32; 2],
    threshold: f32,
    max_search_steps: f32,
}

/// SMAA's three passes plus the two intermediate targets they hand between them.
pub struct SmaaPass {
    edges_pipeline: wgpu::RenderPipeline,
    weights_pipeline: wgpu::RenderPipeline,
    blend_pipeline: wgpu::RenderPipeline,
    bind_group_layout: wgpu::BindGroupLayout,
    sampler: wgpu::Sampler,
    params_buffer: wgpu::Buffer,
    /// 1x1 stand-in bound as `aux_tex` by the two passes that do not read it.
    /// A layout entry with nothing behind it is not a legal bind group, and a
    /// second layout for the sake of one unused binding is worse than a texel.
    dummy_view: wgpu::TextureView,

    /// Tone-mapped LDR image. Post-processing writes here when SMAA is active,
    /// exactly as it writes `FxaaPass::ldr_view` when FXAA is.
    ldr_texture: wgpu::Texture,
    pub ldr_view: wgpu::TextureView,
    edges_texture: wgpu::Texture,
    edges_view: wgpu::TextureView,
    weights_texture: wgpu::Texture,
    weights_view: wgpu::TextureView,

    /// Bind groups for the three passes, rebuilt on resize.
    edges_bg: wgpu::BindGroup,
    weights_bg: wgpu::BindGroup,
    blend_bg: wgpu::BindGroup,

    active: bool,
    threshold: f32,
    max_search_steps: u32,
    width: u32,
    height: u32,
}

const EDGE_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rg8Unorm;
const WEIGHT_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;

impl SmaaPass {
    pub fn new(
        device: &wgpu::Device,
        shaders: &crate::shaders::Shaders,
        surface_format: wgpu::TextureFormat,
        width: u32,
        height: u32,
    ) -> Self {
        let bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("SMAA BGL"),
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
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 3,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
            ],
        });

        // Point sampling for the edge and weight targets — they are not images
        // and interpolating a run-length would invent edges that are not there.
        // The colour taps in the blend pass are all exact texel centres, so
        // linear filtering there is equivalent and costs nothing to allow.
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("SMAA Sampler"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            address_mode_w: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Nearest,
            min_filter: wgpu::FilterMode::Nearest,
            ..Default::default()
        });

        let params_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("SMAA Params"),
            size: std::mem::size_of::<SmaaParams>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let dummy = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("SMAA Dummy"),
            size: wgpu::Extent3d {
                width: 1,
                height: 1,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });
        let dummy_view = dummy.create_view(&wgpu::TextureViewDescriptor::default());

        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("SMAA Shader"),
            source: wgpu::ShaderSource::Wgsl(shaders.source_or_panic("smaa.wgsl").into()),
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("SMAA Pipeline Layout"),
            bind_group_layouts: &[Some(&bind_group_layout)],
            immediate_size: 0,
        });
        let make_pipeline = |label: &str, entry: &str, format: wgpu::TextureFormat| {
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(label),
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
                    entry_point: Some(entry),
                    targets: &[Some(wgpu::ColorTargetState {
                        format,
                        blend: Some(wgpu::BlendState::REPLACE),
                        write_mask: wgpu::ColorWrites::ALL,
                    })],
                    compilation_options: Default::default(),
                }),
                primitive: wgpu::PrimitiveState::default(),
                depth_stencil: None,
                multisample: wgpu::MultisampleState::default(),
                cache: None,
            })
        };
        let edges_pipeline = make_pipeline("SMAA Edges", "fs_edges", EDGE_FORMAT);
        let weights_pipeline = make_pipeline("SMAA Weights", "fs_weights", WEIGHT_FORMAT);
        let blend_pipeline = make_pipeline("SMAA Blend", "fs_blend", surface_format);

        let (ldr_texture, ldr_view) =
            Self::alloc(device, "SMAA LDR", surface_format, width, height);
        let (edges_texture, edges_view) =
            Self::alloc(device, "SMAA Edges", EDGE_FORMAT, width, height);
        let (weights_texture, weights_view) =
            Self::alloc(device, "SMAA Weights", WEIGHT_FORMAT, width, height);

        let edges_bg = Self::bind(
            device,
            &bind_group_layout,
            "SMAA Edges BG",
            &ldr_view,
            &dummy_view,
            &sampler,
            &params_buffer,
        );
        let weights_bg = Self::bind(
            device,
            &bind_group_layout,
            "SMAA Weights BG",
            &edges_view,
            &dummy_view,
            &sampler,
            &params_buffer,
        );
        let blend_bg = Self::bind(
            device,
            &bind_group_layout,
            "SMAA Blend BG",
            &ldr_view,
            &weights_view,
            &sampler,
            &params_buffer,
        );

        Self {
            edges_pipeline,
            weights_pipeline,
            blend_pipeline,
            bind_group_layout,
            sampler,
            params_buffer,
            dummy_view,
            ldr_texture,
            ldr_view,
            edges_texture,
            edges_view,
            weights_texture,
            weights_view,
            edges_bg,
            weights_bg,
            blend_bg,
            active: false,
            // Mirrors `SmaaPreset::Ultra`, which is the authored default.
            threshold: 0.05,
            max_search_steps: 32,
            width,
            height,
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
        label: &str,
        src: &wgpu::TextureView,
        aux: &wgpu::TextureView,
        sampler: &wgpu::Sampler,
        params: &wgpu::Buffer,
    ) -> wgpu::BindGroup {
        device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some(label),
            layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(src),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(sampler),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: params.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: wgpu::BindingResource::TextureView(aux),
                },
            ],
        })
    }

    fn rebuild_bind_groups(&mut self, device: &wgpu::Device) {
        self.edges_bg = Self::bind(
            device,
            &self.bind_group_layout,
            "SMAA Edges BG",
            &self.ldr_view,
            &self.dummy_view,
            &self.sampler,
            &self.params_buffer,
        );
        self.weights_bg = Self::bind(
            device,
            &self.bind_group_layout,
            "SMAA Weights BG",
            &self.edges_view,
            &self.dummy_view,
            &self.sampler,
            &self.params_buffer,
        );
        self.blend_bg = Self::bind(
            device,
            &self.bind_group_layout,
            "SMAA Blend BG",
            &self.ldr_view,
            &self.weights_view,
            &self.sampler,
            &self.params_buffer,
        );
    }

    /// Whether SMAA runs, and the preset's two knobs.
    ///
    /// `threshold` is the relative luma contrast that counts as an edge and
    /// `max_search_steps` how far along one the search walks — the whole of
    /// what a quality preset means here.
    pub fn set_mode(&mut self, active: bool, threshold: f32, max_search_steps: u32) {
        self.active = active;
        self.threshold = threshold;
        self.max_search_steps = max_search_steps;
    }

    /// Whether the morphological passes run this frame.
    #[must_use]
    pub fn active(&self) -> bool {
        self.active
    }

    /// Recreate the three targets at a new size.
    pub fn resize(
        &mut self,
        device: &wgpu::Device,
        surface_format: wgpu::TextureFormat,
        width: u32,
        height: u32,
    ) {
        if self.width == width && self.height == height {
            return;
        }
        let (lt, lv) = Self::alloc(device, "SMAA LDR", surface_format, width, height);
        let (et, ev) = Self::alloc(device, "SMAA Edges", EDGE_FORMAT, width, height);
        let (wt, wv) = Self::alloc(device, "SMAA Weights", WEIGHT_FORMAT, width, height);
        self.ldr_texture = lt;
        self.ldr_view = lv;
        self.edges_texture = et;
        self.edges_view = ev;
        self.weights_texture = wt;
        self.weights_view = wv;
        self.width = width;
        self.height = height;
        self.rebuild_bind_groups(device);
    }

    /// Bytes the two intermediate targets occupy at the current size.
    ///
    /// Reported rather than estimated because `phase_MORROWIND.md` §13 wants a
    /// memory figure per resolution and an assertion is better than a table
    /// somebody retypes.
    #[must_use]
    pub fn intermediate_bytes(&self) -> u64 {
        let px = u64::from(self.width.max(1)) * u64::from(self.height.max(1));
        px * 2 + px * 4
    }

    pub fn update(&self, queue: &wgpu::Queue, width: u32, height: u32) {
        let params = SmaaParams {
            inv_size: [1.0 / width.max(1) as f32, 1.0 / height.max(1) as f32],
            threshold: self.threshold,
            max_search_steps: self.max_search_steps as f32,
        };
        queue.write_buffer(&self.params_buffer, 0, bytemuck::bytes_of(&params));
    }

    /// Resolve `ldr_view` onto `dst`. The caller has already tone-mapped into
    /// `ldr_view`; nothing else is required of it.
    pub fn record(&self, encoder: &mut wgpu::CommandEncoder, dst: &wgpu::TextureView) {
        let mut draw = |label: &str,
                        pipeline: &wgpu::RenderPipeline,
                        bg: &wgpu::BindGroup,
                        target: &wgpu::TextureView,
                        clear: bool| {
            let mut rpass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some(label),
                multiview_mask: None,
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: target,
                    resolve_target: None,
                    depth_slice: None,
                    ops: wgpu::Operations {
                        // The edge and weight targets must start empty: pass 2
                        // reads pass 1's output outside the current pixel, so a
                        // stale texel from the previous frame is a phantom edge
                        // that moves.
                        load: if clear {
                            wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT)
                        } else {
                            wgpu::LoadOp::Clear(wgpu::Color::BLACK)
                        },
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
            });
            rpass.set_pipeline(pipeline);
            rpass.set_bind_group(0, bg, &[]);
            rpass.draw(0..3, 0..1);
        };

        draw(
            "SMAA Edges",
            &self.edges_pipeline,
            &self.edges_bg,
            &self.edges_view,
            true,
        );
        draw(
            "SMAA Weights",
            &self.weights_pipeline,
            &self.weights_bg,
            &self.weights_view,
            true,
        );
        draw(
            "SMAA Blend",
            &self.blend_pipeline,
            &self.blend_bg,
            dst,
            false,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::{EDGE_FORMAT, WEIGHT_FORMAT};

    /// The two intermediates are 6 bytes a pixel between them. Asserted rather
    /// than written in a table so the figure in the record cannot rot.
    #[test]
    fn intermediate_targets_cost_six_bytes_a_pixel() {
        // `block_copy_size`, not `target_pixel_byte_cost`: the latter is
        // wgpu's attachment budget number and reports 8 for `Rgba8Unorm`,
        // which is not how much memory the texture occupies.
        assert_eq!(EDGE_FORMAT.block_copy_size(None), Some(2));
        assert_eq!(WEIGHT_FORMAT.block_copy_size(None), Some(4));
        // 1920x1080 -> 12.4 MB; 3840x2160 -> 49.8 MB. Fixed, unlike a linked
        // list's node pool, which is the whole argument in the AC report.
        assert_eq!(1920u64 * 1080 * 6, 12_441_600);
        assert_eq!(3840u64 * 2160 * 6, 49_766_400);
    }

    /// The coverage solve is the part of SMAA that would otherwise be a
    /// vendored table, so it is the part worth pinning. These mirror
    /// `smaa_coverage` in `smaa.wgsl`; the shader is validated separately by
    /// `shaders_validate`, and this is the arithmetic contract it implements.
    fn coverage(d1: f32, d2: f32, s1: f32, s2: f32) -> f32 {
        let len = d1 + d2 + 1.0;
        let t = (d1 + 0.5) / len;
        let offset = (s1 + (s2 - s1) * t) * 0.5;
        offset.clamp(0.0, 0.5)
    }

    #[test]
    fn a_clean_step_blends_half_along_its_whole_length() {
        // Both ends turn the same way: the silhouette is parallel to the edge,
        // offset by half a pixel everywhere.
        for d1 in 0..8 {
            let d2 = 7 - d1;
            let c = coverage(d1 as f32, d2 as f32, 1.0, 1.0);
            assert!((c - 0.5).abs() < 1e-6, "d1={d1} gave {c}");
        }
    }

    #[test]
    fn a_diagonal_ramps_through_zero_and_never_goes_negative() {
        // Opposite turns: the silhouette crosses the boundary in the middle, so
        // this side is covered on one half and not the other. Clamping at zero
        // is what stops a pixel pulling colour from the wrong neighbour.
        let mut seen_zero = false;
        let mut seen_positive = false;
        for d1 in 0..8 {
            let c = coverage(d1 as f32, (7 - d1) as f32, -1.0, 1.0);
            assert!((0.0..=0.5).contains(&c), "d1={d1} gave {c}");
            seen_zero |= c == 0.0;
            seen_positive |= c > 0.0;
        }
        assert!(seen_zero && seen_positive, "a diagonal should do both");
    }

    #[test]
    fn one_open_end_ramps_to_zero() {
        // A run the search could not terminate contributes nothing at that end,
        // which is what keeps a long unresolved edge from being blended flat.
        let near = coverage(0.0, 7.0, 1.0, 0.0);
        let far = coverage(7.0, 0.0, 1.0, 0.0);
        assert!(near > far, "coverage should fall away from the turning end");
        assert!(far >= 0.0);
    }

    #[test]
    fn an_unterminated_run_contributes_nothing() {
        for d1 in 0..8 {
            assert_eq!(coverage(d1 as f32, (7 - d1) as f32, 0.0, 0.0), 0.0);
        }
    }


}
