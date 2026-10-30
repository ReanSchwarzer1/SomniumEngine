//! Editor selection and hover glow (Phase ALYX-J).
//!
//! The masks come from two [`GameHighlightPass`](super::game_highlight::GameHighlightPass) instances, so the editor
//! outlines exactly what is visible, the way the game's interaction outline does. This pass only composites them.
//! Scene draws are matched by entity: by the convention every scene submitter follows, the low 32 bits of a draw's
//! [`SortKey`](crate::command::SortKey) are the entity's index.
use std::collections::HashSet;

use super::game_highlight::HighlightDraw;
use crate::command::DrawCommand;

/// How the glow looks. The defaults are the editor's.
#[derive(Clone, Copy, Debug)]
pub struct GlowStyle {
    /// Selection colour (linear) and strength.
    pub selection: [f32; 4],
    /// Hover colour and strength.
    pub hover: [f32; 4],
    /// The line hugging the shape, in output pixels.
    pub line_px: f32,
    /// How far the selection's glow reaches, in output pixels.
    pub glow_px: f32,
    /// The wash of selection colour inside the shape.
    pub fill: f32,
}

impl Default for GlowStyle {
    fn default() -> Self {
        Self {
            // Warm amber: the colour every engine's users already read as "selected", and one no scene light in a
            // dark level competes with.
            selection: [1.0, 0.62, 0.16, 1.0],
            // A pale, cool line: clearly not the selection.
            hover: [0.78, 0.84, 1.0, 0.85],
            line_px: 2.0,
            glow_px: 7.0,
            fill: 0.07,
        }
    }
}

/// Which instances belong to `entities`: per opaque instance a 0/1 flag, and the transparent draws to cover.
///
/// The key that identifies a scene entity is the sort key's low 32 bits; `excluded` vertex offsets (terrain chunks,
/// keyed by their offset instead) never match, and neither does an all-zero key (foliage batches use one).
pub(crate) fn entity_members(
    opaque: &[DrawCommand],
    shadow_only_count: usize,
    transparent: &[DrawCommand],
    entities: &HashSet<u32>,
    excluded: impl Fn(u32) -> bool,
) -> (Vec<u32>, Vec<HighlightDraw>) {
    let member = |cmd: &DrawCommand| {
        cmd.sort_key.0 != 0
            && !excluded(cmd.vertex_offset)
            && entities.contains(&(cmd.sort_key.0 as u32))
    };
    let members = opaque.iter().map(|cmd| u32::from(member(cmd))).collect();
    let base = opaque.len() + shadow_only_count;
    let draws = transparent
        .iter()
        .enumerate()
        .filter(|(_, cmd)| member(cmd))
        .map(|(i, cmd)| HighlightDraw {
            instance_index: u32::try_from(base + i).expect("instance range exceeds u32"),
            index_count: cmd.index_count,
        })
        .collect();
    (members, draws)
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct GlowParams {
    selection: [f32; 4],
    hover: [f32; 4],
    inverse_output: [f32; 2],
    line_px: f32,
    glow_px: f32,
    source_uv_offset: [f32; 2],
    fill: f32,
    has_hover: f32,
}

pub struct EditorGlowPass {
    pipeline: wgpu::RenderPipeline,
    layout: wgpu::BindGroupLayout,
    sampler: wgpu::Sampler,
    params: wgpu::Buffer,
    /// Stands in for a mask there is none of this frame, so one bind group layout serves every case.
    blank: wgpu::TextureView,
}

impl EditorGlowPass {
    pub fn new(device: &wgpu::Device, shaders: &crate::shaders::Shaders, format: wgpu::TextureFormat) -> Self {
        let texture = |binding| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Texture {
                sample_type: wgpu::TextureSampleType::Float { filterable: true },
                view_dimension: wgpu::TextureViewDimension::D2,
                multisampled: false,
            },
            count: None,
        };
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("Editor glow layout"),
            entries: &[
                texture(0),
                texture(1),
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 3,
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
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("Editor glow shader"),
            source: wgpu::ShaderSource::Wgsl(shaders.source_or_panic("editor_glow.wgsl").into()),
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("Editor glow pipeline layout"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("Editor glow"),
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
                    format,
                    blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
                compilation_options: Default::default(),
            }),
            primitive: Default::default(),
            depth_stencil: None,
            multisample: Default::default(),
            cache: None,
        });
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("Editor glow sampler"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        let params = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Editor glow parameters"),
            size: std::mem::size_of::<GlowParams>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let blank = device
            .create_texture(&wgpu::TextureDescriptor {
                label: Some("Editor glow blank mask"),
                size: wgpu::Extent3d { width: 1, height: 1, depth_or_array_layers: 1 },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::R8Unorm,
                usage: wgpu::TextureUsages::TEXTURE_BINDING,
                view_formats: &[],
            })
            .create_view(&Default::default());
        Self { pipeline, layout, sampler, params, blank }
    }

    /// Composite the two masks over `output`. A missing mask (nothing selected, nothing hovered) draws nothing of
    /// its kind.
    #[allow(clippy::too_many_arguments)]
    pub fn composite(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        output: &wgpu::TextureView,
        output_extent: [u32; 2],
        rect: Option<(u32, u32, u32, u32)>,
        selection: Option<&wgpu::TextureView>,
        hover: Option<&wgpu::TextureView>,
        style: GlowStyle,
    ) {
        if selection.is_none() && hover.is_none() {
            return;
        }
        let size = rect.map_or(output_extent, |(_, _, w, h)| [w, h]);
        if size.contains(&0) {
            return;
        }
        let params = GlowParams {
            selection: if selection.is_some() { style.selection } else { [0.0; 4] },
            hover: style.hover,
            inverse_output: [1.0 / size[0] as f32, 1.0 / size[1] as f32],
            line_px: style.line_px,
            glow_px: style.glow_px,
            source_uv_offset: [0.0; 2],
            fill: style.fill,
            has_hover: f32::from(u8::from(hover.is_some())),
        };
        queue.write_buffer(&self.params, 0, bytemuck::bytes_of(&params));
        let bindings = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("Editor glow bindings"),
            layout: &self.layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(selection.unwrap_or(&self.blank)),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(hover.unwrap_or(&self.blank)),
                },
                wgpu::BindGroupEntry { binding: 2, resource: wgpu::BindingResource::Sampler(&self.sampler) },
                wgpu::BindGroupEntry { binding: 3, resource: self.params.as_entire_binding() },
            ],
        });
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("Editor glow"),
            multiview_mask: None,
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: output,
                resolve_target: None,
                depth_slice: None,
                ops: wgpu::Operations { load: wgpu::LoadOp::Load, store: wgpu::StoreOp::Store },
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
        });
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, &bindings, &[]);
        if let Some((x, y, w, h)) = rect {
            pass.set_viewport(x as f32, y as f32, w as f32, h as f32, 0.0, 1.0);
            pass.set_scissor_rect(x, y, w, h);
        }
        pass.draw(0..3, 0..1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::command::SortKey;

    fn draw(entity: u32, vertex_offset: u32) -> DrawCommand {
        DrawCommand {
            sort_key: SortKey::new(0, 3, entity),
            vertex_offset,
            index_offset: 0,
            index_count: 6,
            material_id: 3,
            transform: glam::Mat4::IDENTITY,
            casts_shadow: true,
            paint: 0,
        }
    }

    #[test]
    fn members_follow_the_entity_in_the_sort_key_and_skip_terrain_and_foliage() {
        let selected = HashSet::from([7u32, 0u32]);
        let opaque = vec![
            draw(7, 100),
            draw(8, 200),
            // A terrain chunk keyed by its vertex offset, which happens to equal a selected index.
            draw(7, 7),
            // A foliage batch: the all-zero key, which would otherwise match entity 0.
            DrawCommand { sort_key: SortKey(0), ..draw(0, 300) },
        ];
        let glass = vec![draw(7, 400), draw(9, 500)];
        let (members, transparent) = entity_members(&opaque, 2, &glass, &selected, |offset| offset == 7);
        assert_eq!(members, vec![1, 0, 0, 0]);
        assert_eq!(transparent.len(), 1);
        assert_eq!(transparent[0].instance_index, 6, "after the opaque and shadow-only instances");
    }

    #[test]
    fn parameters_match_the_shader_layout() {
        assert_eq!(std::mem::size_of::<GlowParams>(), 64);
    }
}
