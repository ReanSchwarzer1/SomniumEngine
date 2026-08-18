// UiPass — wgpu render pass for native UI draw lists.
//
// Phase 12B-1 uploaded a vertex/index pair and issued one indexed draw per
// DrawCommand. Phase 27-A (Styx) replaced that with a single instanced
// pipeline: the unit quad is generated in the vertex stage from
// `@builtin(vertex_index)`, so the only per-frame upload is the `Primitive`
// instance list, and each DrawCommand becomes one `draw(0..6, instances)`.
//
// Bind groups:
//   BG0 b0 (VERTEX|FRAGMENT): Globals — mat4 ortho projection + text gamma.
//   BG1 (FRAGMENT): b0 font atlas, b1 icon atlas, b2 shared sampler. Bound once
//     for the whole pass; each instance names its own atlas in the TEX_* bits of
//     `Primitive::flags`. The pre-Styx pass rebound BG1 whenever the texture
//     changed between commands, which measured 164 draw calls for 625 quads on
//     the real 1920x1080 shell. The white 1x1 texture is gone with them: an
//     untextured primitive takes no sample at all.

use crate::{
    draw::{DrawCommand, DrawingContext},
    font::{ATLAS_HEIGHT, ATLAS_WIDTH},
    icons::{ICON_ATLAS_HEIGHT, ICON_ATLAS_WIDTH},
    primitive::Primitive,
};
use glam::Mat4;
use std::borrow::Cow;

/// 16 K instances before the first grow. Styx emits roughly one instance where
/// the pre-Styx list emitted four vertices plus six indices, so this is a
/// smaller allocation than the buffers it replaces.
const INIT_INSTANCE_CAP: u64 = 16384 * 100;

/// Six vertices per instance: two triangles over the unit quad.
const VERTS_PER_QUAD: u32 = 6;

const SRGB_OUTPUT_DECLARATION: &str = "const OUTPUT_IS_SRGB: bool = true;";

/// Exponent applied to glyph coverage before blending (Phase 27-B).
///
/// The pipeline blends in linear space on an sRGB target, which is correct for
/// colour but renders light-on-dark stems heavier and softer than the
/// rasterizer intended, because grayscale antialiasing coverage is authored for
/// perceptual blending. Raising coverage to a power above 1 thins the stems
/// back. There is no framebuffer read in this pass, so the background luminance
/// is unknown and an exact per-pixel correction is impossible; this is a single
/// constant tuned for the Nocturne ground.
///
/// **Empirical.** `dev records/phase_27.md` §17 records it as needing
/// capture-based tuning; [`UiPass::set_text_gamma`] exists so that can happen
/// without a shader edit, and 1.0 reproduces pre-Styx text exactly.
pub const DEFAULT_TEXT_GAMMA: f32 = 1.18;

// Thinning, never fattening: a value below 1.0 would make the problem worse.
const _: () = assert!(DEFAULT_TEXT_GAMMA > 1.0);

fn shader_source_for_surface_format(format: wgpu::TextureFormat) -> Cow<'static, str> {
    let source = include_str!("ui_pass.wgsl");
    if format.is_srgb() {
        Cow::Borrowed(source)
    } else {
        debug_assert!(source.contains(SRGB_OUTPUT_DECLARATION));
        Cow::Owned(source.replace(
            SRGB_OUTPUT_DECLARATION,
            "const OUTPUT_IS_SRGB: bool = false;",
        ))
    }
}

/// BG0 uniform. Matches the `Globals` struct in `ui_pass.wgsl`: a mat4x4 is 64
/// bytes at align 16, so the three trailing pads bring the total to 80.
#[repr(C)]
#[derive(Clone, Copy, Debug, bytemuck::Pod, bytemuck::Zeroable)]
struct Globals {
    proj: [f32; 16],
    text_gamma: f32,
    _pad: [f32; 3],
}

const _: () = assert!(std::mem::size_of::<Globals>() == 80);

/// Per-frame counters for the Phase 27-I performance harness.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct UiFrameStats {
    pub instances: u32,
    pub batches: u32,
    pub instance_bytes: u32,
}

pub struct UiPass {
    pipeline: wgpu::RenderPipeline,
    bg1_layout: wgpu::BindGroupLayout,
    // BG0 — globals uniform
    globals_buf: wgpu::Buffer,
    bg0: wgpu::BindGroup,
    // Shared sampler
    sampler: wgpu::Sampler,
    // BG1 — both atlases, bound once per pass
    atlas_tex: wgpu::Texture,
    atlas_view: wgpu::TextureView,
    icon_tex: wgpu::Texture,
    icon_view: wgpu::TextureView,
    bg1: wgpu::BindGroup,
    // Instance buffer (recreated on overflow)
    inst_buf: wgpu::Buffer,
    inst_capacity: u64,
    // Draw list cached from the last prepare() call
    commands: Vec<DrawCommand>,
    text_gamma: f32,
    stats: UiFrameStats,
    surface_w: u32,
    surface_h: u32,
}

impl UiPass {
    /// `queue` is retained in the signature for call-site compatibility. Styx
    /// no longer uploads anything at construction — the white 1x1 texture it
    /// used to seed here was retired when untextured primitives stopped
    /// sampling — and the atlases upload lazily in [`Self::prepare`].
    pub fn new(
        device: &wgpu::Device,
        _queue: &wgpu::Queue,
        surface_format: wgpu::TextureFormat,
    ) -> Self {
        // ── Bind group layouts ────────────────────────────────────────────────
        let bg0_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("UiPass BGL0"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                // The fragment stage reads `text_gamma` from the same uniform.
                visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            }],
        });

        let atlas_entry = |binding: u32| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Texture {
                sample_type: wgpu::TextureSampleType::Float { filterable: true },
                view_dimension: wgpu::TextureViewDimension::D2,
                multisampled: false,
            },
            count: None,
        };
        let bg1_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("UiPass BGL1"),
            entries: &[
                atlas_entry(0),
                atlas_entry(1),
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });

        // ── Globals uniform buffer ────────────────────────────────────────────
        let globals_buf = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("UiPass Globals"),
            size: std::mem::size_of::<Globals>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let bg0 = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("UiPass BG0"),
            layout: &bg0_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: globals_buf.as_entire_binding(),
            }],
        });

        // ── Shared linear sampler ─────────────────────────────────────────────
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("UiPass Sampler"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });

        // ── Font atlas texture ────────────────────────────────────────────────
        let atlas_tex = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("UiPass Font Atlas"),
            size: wgpu::Extent3d {
                width: ATLAS_WIDTH,
                height: ATLAS_HEIGHT,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        let atlas_view = atlas_tex.create_view(&wgpu::TextureViewDescriptor::default());

        let icon_tex = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("UiPass Icon Atlas"),
            size: wgpu::Extent3d {
                width: ICON_ATLAS_WIDTH,
                height: ICON_ATLAS_HEIGHT,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        let icon_view = icon_tex.create_view(&wgpu::TextureViewDescriptor::default());

        // ── BG1 bind groups ───────────────────────────────────────────────────
        let bg1 = Self::make_bg1(device, &bg1_layout, &atlas_view, &icon_view, &sampler);

        // ── Instance buffer ───────────────────────────────────────────────────
        let inst_buf = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("UI Instances"),
            size: INIT_INSTANCE_CAP,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        // ── Shader + pipeline ─────────────────────────────────────────────────
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("UiPass Shader"),
            source: wgpu::ShaderSource::Wgsl(shader_source_for_surface_format(surface_format)),
        });

        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("UiPass Pipeline Layout"),
            bind_group_layouts: &[Some(&bg0_layout), Some(&bg1_layout)],
            immediate_size: 0,
        });

        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("UiPass Pipeline"),
            layout: Some(&pipeline_layout),
            multiview_mask: None,
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                compilation_options: Default::default(),
                buffers: &[wgpu::VertexBufferLayout {
                    array_stride: Primitive::STRIDE,
                    step_mode: wgpu::VertexStepMode::Instance,
                    attributes: &Primitive::VERTEX_ATTRS,
                }],
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_main"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: surface_format,
                    // Straight (unassociated) alpha, unchanged from Phase 12B-1.
                    blend: Some(wgpu::BlendState {
                        color: wgpu::BlendComponent {
                            src_factor: wgpu::BlendFactor::SrcAlpha,
                            dst_factor: wgpu::BlendFactor::OneMinusSrcAlpha,
                            operation: wgpu::BlendOperation::Add,
                        },
                        alpha: wgpu::BlendComponent {
                            src_factor: wgpu::BlendFactor::One,
                            dst_factor: wgpu::BlendFactor::OneMinusSrcAlpha,
                            operation: wgpu::BlendOperation::Add,
                        },
                    }),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                front_face: wgpu::FrontFace::Ccw,
                cull_mode: None,
                ..Default::default()
            },
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            cache: None,
        });

        Self {
            pipeline,
            bg1_layout,
            globals_buf,
            bg0,
            sampler,
            atlas_tex,
            atlas_view,
            icon_tex,
            icon_view,
            bg1,
            inst_buf,
            inst_capacity: INIT_INSTANCE_CAP,
            commands: Vec::new(),
            text_gamma: DEFAULT_TEXT_GAMMA,
            stats: UiFrameStats::default(),
            surface_w: 0,
            surface_h: 0,
        }
    }

    fn make_bg1(
        device: &wgpu::Device,
        layout: &wgpu::BindGroupLayout,
        font_view: &wgpu::TextureView,
        icon_view: &wgpu::TextureView,
        sampler: &wgpu::Sampler,
    ) -> wgpu::BindGroup {
        device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("UiPass BG1"),
            layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(font_view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(icon_view),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::Sampler(sampler),
                },
            ],
        })
    }

    /// Glyph-coverage exponent. See [`DEFAULT_TEXT_GAMMA`]. Setting 1.0
    /// reproduces pre-Styx text exactly, which is how the 27-B change is
    /// isolated in a capture diff.
    pub fn set_text_gamma(&mut self, gamma: f32) {
        self.text_gamma = gamma.clamp(0.5, 2.5);
    }

    /// Counters from the last [`Self::prepare`] call.
    pub fn stats(&self) -> UiFrameStats {
        self.stats
    }

    /// Upload draw data to the GPU. Call once per frame before `render()`.
    pub fn prepare(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        draw_ctx: &mut DrawingContext,
        surface_w: u32,
        surface_h: u32,
    ) {
        self.surface_w = surface_w;
        self.surface_h = surface_h;

        // Ortho: (0,0) = top-left, (W,H) = bottom-right, y-down.
        let proj = Mat4::orthographic_rh(0.0, surface_w as f32, surface_h as f32, 0.0, 0.0, 1.0);
        let globals = Globals {
            proj: proj.to_cols_array(),
            text_gamma: self.text_gamma,
            _pad: [0.0; 3],
        };
        queue.write_buffer(&self.globals_buf, 0, bytemuck::bytes_of(&globals));

        // Font atlas — dirty flag cleared here so we don't re-upload next frame.
        let mut atlases_changed = false;
        if draw_ctx.font_atlas.dirty {
            queue.write_texture(
                wgpu::TexelCopyTextureInfo {
                    texture: &self.atlas_tex,
                    mip_level: 0,
                    origin: wgpu::Origin3d::ZERO,
                    aspect: wgpu::TextureAspect::All,
                },
                &draw_ctx.font_atlas.pixels,
                wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(ATLAS_WIDTH * 4),
                    rows_per_image: Some(ATLAS_HEIGHT),
                },
                wgpu::Extent3d {
                    width: ATLAS_WIDTH,
                    height: ATLAS_HEIGHT,
                    depth_or_array_layers: 1,
                },
            );
            draw_ctx.font_atlas.dirty = false;
            atlases_changed = true;
        }

        if draw_ctx.icon_atlas.dirty {
            queue.write_texture(
                wgpu::TexelCopyTextureInfo {
                    texture: &self.icon_tex,
                    mip_level: 0,
                    origin: wgpu::Origin3d::ZERO,
                    aspect: wgpu::TextureAspect::All,
                },
                &draw_ctx.icon_atlas.pixels,
                wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(ICON_ATLAS_WIDTH * 4),
                    rows_per_image: Some(ICON_ATLAS_HEIGHT),
                },
                wgpu::Extent3d {
                    width: ICON_ATLAS_WIDTH,
                    height: ICON_ATLAS_HEIGHT,
                    depth_or_array_layers: 1,
                },
            );
            draw_ctx.icon_atlas.dirty = false;
            atlases_changed = true;
        }

        // One bind group serves the whole pass, so it is rebuilt only when an
        // atlas texture is replaced.
        if atlases_changed {
            self.bg1 = Self::make_bg1(
                device,
                &self.bg1_layout,
                &self.atlas_view,
                &self.icon_view,
                &self.sampler,
            );
        }

        // Instances
        let instance_bytes = (draw_ctx.instances.len() * std::mem::size_of::<Primitive>()) as u64;
        if !draw_ctx.instances.is_empty() {
            if instance_bytes > self.inst_capacity {
                self.inst_capacity = instance_bytes * 2;
                self.inst_buf = device.create_buffer(&wgpu::BufferDescriptor {
                    label: Some("UI Instances"),
                    size: self.inst_capacity,
                    usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
                    mapped_at_creation: false,
                });
            }
            queue.write_buffer(&self.inst_buf, 0, bytemuck::cast_slice(&draw_ctx.instances));
        }

        self.commands.clear();
        self.commands.extend_from_slice(&draw_ctx.commands);
        self.stats = UiFrameStats {
            instances: draw_ctx.instances.len() as u32,
            batches: draw_ctx
                .commands
                .iter()
                .filter(|c| c.instance_count > 0)
                .count() as u32,
            instance_bytes: instance_bytes as u32,
        };
    }

    /// Record the UI render pass. Composites onto the existing surface contents.
    pub fn render(&self, encoder: &mut wgpu::CommandEncoder, surface_view: &wgpu::TextureView) {
        if self.commands.is_empty() {
            return;
        }

        let mut rpass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("UiPass"),
            multiview_mask: None,
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: surface_view,
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

        rpass.set_pipeline(&self.pipeline);
        rpass.set_vertex_buffer(0, self.inst_buf.slice(..));
        rpass.set_bind_group(0, &self.bg0, &[]);

        // Bound once: every instance names its own atlas.
        rpass.set_bind_group(1, &self.bg1, &[]);

        let sw = self.surface_w;
        let sh = self.surface_h;

        for cmd in &self.commands {
            if cmd.instance_count == 0 {
                continue;
            }

            // Scissor rect clamped to [0, surface_w) × [0, surface_h).
            let x0 = (cmd.clip_rect.x.max(0.0) as u32).min(sw);
            let y0 = (cmd.clip_rect.y.max(0.0) as u32).min(sh);
            let x1 = ((cmd.clip_rect.x + cmd.clip_rect.w).max(0.0) as u32).min(sw);
            let y1 = ((cmd.clip_rect.y + cmd.clip_rect.h).max(0.0) as u32).min(sh);
            let cw = x1.saturating_sub(x0);
            let ch = y1.saturating_sub(y0);
            if cw == 0 || ch == 0 {
                continue;
            }
            rpass.set_scissor_rect(x0, y0, cw, ch);

            rpass.draw(
                0..VERTS_PER_QUAD,
                cmd.instance_offset..(cmd.instance_offset + cmd.instance_count),
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn srgb_surface_decodes_authored_vertex_tints() {
        let shader = shader_source_for_surface_format(wgpu::TextureFormat::Bgra8UnormSrgb);
        assert!(shader.contains("const OUTPUT_IS_SRGB: bool = true;"));
    }

    #[test]
    fn linear_surface_does_not_apply_an_extra_decode() {
        let shader = shader_source_for_surface_format(wgpu::TextureFormat::Bgra8Unorm);
        assert!(shader.contains("const OUTPUT_IS_SRGB: bool = false;"));
    }

    #[test]
    fn shader_decodes_srgb_exactly_once() {
        // phase_27 §6.2: the decode must appear in exactly one function, and
        // every colour path must route through it rather than inlining a second
        // transfer function.
        let shader = shader_source_for_surface_format(wgpu::TextureFormat::Bgra8UnormSrgb);
        assert_eq!(
            shader.matches("fn decode_srgb").count(),
            1,
            "exactly one decode helper may exist"
        );
        assert_eq!(
            shader.matches("1.055").count(),
            1,
            "the sRGB transfer constants may appear only inside decode_srgb"
        );
        assert_eq!(shader.matches("0.04045").count(), 1);
        assert_eq!(shader.matches("12.92").count(), 1);
    }

    #[test]
    fn shader_decodes_before_gradient_interpolation() {
        // Mixing authored sRGB and then decoding would make a 50 % stop the
        // sRGB mean instead of the linear mean.
        let shader = shader_source_for_surface_format(wgpu::TextureFormat::Bgra8UnormSrgb);
        let mix_line = shader
            .lines()
            .find(|l| l.contains("mix(decode_srgb"))
            .expect("gradient must mix already-decoded values");
        assert!(mix_line.contains("in.fill_a.rgb"));
        assert!(mix_line.contains("in.fill_b.rgb"));
        assert!(
            !shader.contains("decode_srgb(mix("),
            "decoding after the mix would interpolate in the wrong space"
        );
    }

    #[test]
    fn globals_layout_matches_the_wgsl_struct() {
        assert_eq!(std::mem::size_of::<Globals>(), 80);
        assert_eq!(std::mem::align_of::<Globals>(), 4);
        let shader = shader_source_for_surface_format(wgpu::TextureFormat::Bgra8UnormSrgb);
        assert!(shader.contains("proj: mat4x4<f32>"));
        assert!(shader.contains("text_gamma: f32"));
    }

    #[test]
    fn declared_attribute_formats_exactly_tile_the_instance() {
        // Sum the *format* sizes rather than reading the last offset: this is
        // what catches a gap or an overlap in the middle of the layout, which
        // would make the shader read one field as another.
        let total: u64 = Primitive::VERTEX_ATTRS
            .iter()
            .map(|a| a.format.size())
            .sum();
        assert_eq!(total, Primitive::STRIDE, "attributes must tile the stride");

        // And every attribute must start exactly where the previous one ended.
        let mut cursor = 0u64;
        for attr in Primitive::VERTEX_ATTRS {
            assert_eq!(attr.offset, cursor, "gap or overlap at {attr:?}");
            cursor += attr.format.size();
        }
    }

    #[test]
    fn text_gamma_thins_partial_coverage_and_preserves_the_extremes() {
        // Direction: above 1.0 lowers partial coverage, which is what
        // compensates for linear-space blending of light-on-dark text. The
        // direction itself is a compile-time invariant (see the `const _`
        // above); this covers the behaviour across the coverage range.
        let g = std::hint::black_box(DEFAULT_TEXT_GAMMA);
        for cov in [0.25f32, 0.5, 0.75] {
            assert!(
                cov.powf(g) < cov,
                "coverage {cov} must be thinned, got {}",
                cov.powf(g)
            );
        }
        // A fully covered or fully empty texel must not move, or glyph
        // interiors would lighten and the background would tint.
        assert_eq!(0.0f32.powf(g), 0.0);
        assert_eq!(1.0f32.powf(g), 1.0);
    }
}
