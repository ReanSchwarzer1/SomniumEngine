//! Game-only silhouette feedback from visible fragments, independent of editor selection.
use wgpu::util::DeviceExt;

#[derive(Clone, Copy, Debug)]
pub struct HighlightStyle {
    pub color: [f32; 4],
    pub width_pixels: f32,
    pub show_mask: bool,
}

impl Default for HighlightStyle {
    fn default() -> Self {
        Self {
            color: [0.94, 0.80, 0.36, 0.85],
            width_pixels: 2.0,
            show_mask: false,
        }
    }
}

impl HighlightStyle {
    pub fn validated(self) -> Option<Self> {
        if !self.color.iter().all(|v| v.is_finite())
            || !self.width_pixels.is_finite()
            || self.color[3] <= 0.0
        {
            return None;
        }
        Some(Self {
            color: self.color.map(|v| v.clamp(0.0, 1.0)),
            width_pixels: self.width_pixels.clamp(0.5, 6.0),
            show_mask: self.show_mask,
        })
    }
}

#[derive(Clone, Copy, Debug)]
pub struct HighlightDraw {
    pub instance_index: u32,
    pub index_count: u32,
}

pub(crate) fn frame_members(
    opaque: &[crate::command::DrawCommand],
    shadow_only_count: usize,
    transparent: &[crate::command::DrawCommand],
    selected: &std::collections::HashSet<crate::pass::taa::ReactiveDrawKey>,
) -> (Vec<u32>, Vec<HighlightDraw>) {
    let members = opaque
        .iter()
        .map(|cmd| u32::from(selected.contains(&crate::pass::taa::ReactiveDrawKey::of(cmd))))
        .collect();
    let base = opaque
        .len()
        .checked_add(shadow_only_count)
        .expect("instance range overflow");
    let draws = transparent
        .iter()
        .enumerate()
        .filter(|(_, cmd)| selected.contains(&crate::pass::taa::ReactiveDrawKey::of(cmd)))
        .map(|(i, cmd)| HighlightDraw {
            instance_index: u32::try_from(base + i).expect("instance range exceeds u32"),
            index_count: cmd.index_count,
        })
        .collect();
    (members, draws)
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct HighlightParams {
    color: [f32; 4],
    inverse_output: [f32; 2],
    width_pixels: f32,
    show_mask: f32,
    dream: [f32; 4],
    frame: [f32; 2],
    time: f32,
    _pad: f32,
    source_uv_offset: [f32; 2],
    _pad2: [f32; 2],
}

struct ViewBuffers {
    members: wgpu::Buffer,
    params: wgpu::Buffer,
}

pub struct GameHighlightPass {
    opaque: wgpu::RenderPipeline,
    transparent: wgpu::RenderPipeline,
    composite: wgpu::RenderPipeline,
    mask_layout: wgpu::BindGroupLayout,
    composite_layout: wgpu::BindGroupLayout,
    sampler: wgpu::Sampler,
    material_sampler: wgpu::Sampler,
    mask: Option<(wgpu::Texture, wgpu::TextureView)>,
    extent: [u32; 2],
    views: Vec<ViewBuffers>,
}

impl GameHighlightPass {
    pub fn new(
        device: &wgpu::Device,
        shaders: &crate::shaders::Shaders,
        global_layout: &wgpu::BindGroupLayout,
        format: wgpu::TextureFormat,
        width: u32,
        height: u32,
    ) -> Self {
        let texture = |binding, sample_type| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Texture {
                sample_type,
                view_dimension: wgpu::TextureViewDimension::D2,
                multisampled: false,
            },
            count: None,
        };
        let sampler_entry = |binding| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
            count: None,
        };
        let mask_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("Game highlight mask layout"),
            entries: &[
                texture(0, wgpu::TextureSampleType::Uint),
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Storage { read_only: true },
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                sampler_entry(2),
            ],
        });
        let composite_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("Game highlight composite layout"),
            entries: &[
                texture(0, wgpu::TextureSampleType::Float { filterable: true }),
                sampler_entry(1),
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
            ],
        });
        let mask_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("Game highlight mask shader"),
            source: wgpu::ShaderSource::Wgsl(
                shaders.source_or_panic("game_highlight_mask.wgsl").into(),
            ),
        });
        let composite_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("Game highlight composite shader"),
            source: wgpu::ShaderSource::Wgsl(shaders.source_or_panic("game_highlight.wgsl").into()),
        });
        let mask_pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("Game highlight mask pipeline layout"),
            bind_group_layouts: &[Some(global_layout), Some(&mask_layout)],
            immediate_size: 0,
        });
        let composite_pipeline_layout =
            device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("Game highlight composite pipeline layout"),
                bind_group_layouts: &[Some(&composite_layout)],
                immediate_size: 0,
            });
        let mask_pipeline = |transparent| {
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(if transparent {
                    "Game highlight transparent mask"
                } else {
                    "Game highlight opaque mask"
                }),
                layout: Some(&mask_pipeline_layout),
                multiview_mask: None,
                vertex: wgpu::VertexState {
                    module: &mask_shader,
                    entry_point: Some(if transparent {
                        "vs_transparent"
                    } else {
                        "vs_fullscreen"
                    }),
                    buffers: &[],
                    compilation_options: Default::default(),
                },
                fragment: Some(wgpu::FragmentState {
                    module: &mask_shader,
                    entry_point: Some(if transparent {
                        "fs_transparent"
                    } else {
                        "fs_opaque"
                    }),
                    targets: &[Some(wgpu::ColorTargetState {
                        format: wgpu::TextureFormat::R8Unorm,
                        blend: None,
                        write_mask: wgpu::ColorWrites::RED,
                    })],
                    compilation_options: Default::default(),
                }),
                primitive: wgpu::PrimitiveState {
                    cull_mode: None,
                    ..Default::default()
                },
                depth_stencil: transparent.then_some(wgpu::DepthStencilState {
                    format: wgpu::TextureFormat::Depth32Float,
                    depth_write_enabled: Some(false),
                    depth_compare: Some(wgpu::CompareFunction::LessEqual),
                    stencil: Default::default(),
                    bias: Default::default(),
                }),
                multisample: Default::default(),
                cache: None,
            })
        };
        let opaque = mask_pipeline(false);
        let transparent = mask_pipeline(true);
        let composite = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("Game highlight border"),
            layout: Some(&composite_pipeline_layout),
            multiview_mask: None,
            vertex: wgpu::VertexState {
                module: &composite_shader,
                entry_point: Some("vs_main"),
                buffers: &[],
                compilation_options: Default::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &composite_shader,
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
            label: Some("Game highlight sampler"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        let material_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("Game highlight material sampler"),
            address_mode_u: wgpu::AddressMode::Repeat,
            address_mode_v: wgpu::AddressMode::Repeat,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::MipmapFilterMode::Linear,
            ..Default::default()
        });
        Self {
            opaque,
            transparent,
            composite,
            mask_layout,
            composite_layout,
            sampler,
            material_sampler,
            mask: None,
            extent: [width.max(1), height.max(1)],
            views: Vec::new(),
        }
    }

    pub fn resize(&mut self, width: u32, height: u32) {
        let extent = [width.max(1), height.max(1)];
        if extent != self.extent {
            self.extent = extent;
            self.mask = None;
        }
    }

    fn ensure_view(&mut self, device: &wgpu::Device, slot: usize, member_count: usize) {
        while self.views.len() <= slot {
            self.views.push(ViewBuffers {
                members: device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                    label: Some("Game highlight members"),
                    contents: &[0; 16],
                    usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
                }),
                params: device.create_buffer(&wgpu::BufferDescriptor {
                    label: Some("Game highlight parameters"),
                    size: std::mem::size_of::<HighlightParams>() as u64,
                    usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                    mapped_at_creation: false,
                }),
            });
        }
        let bytes = (member_count.max(1) as u64 * 4).max(16).next_power_of_two();
        if self.views[slot].members.size() < bytes {
            self.views[slot].members = device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("Game highlight members"),
                size: bytes,
                usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
        }
        if self.mask.is_none() {
            let texture = device.create_texture(&wgpu::TextureDescriptor {
                label: Some("Game highlight visible union"),
                size: wgpu::Extent3d {
                    width: self.extent[0],
                    height: self.extent[1],
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::R8Unorm,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                    | wgpu::TextureUsages::TEXTURE_BINDING,
                view_formats: &[],
            });
            let view = texture.create_view(&Default::default());
            self.mask = Some((texture, view));
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub fn record_mask(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        global: &wgpu::BindGroup,
        visibility: &wgpu::TextureView,
        depth: &wgpu::TextureView,
        members: &[u32],
        transparent: &[HighlightDraw],
        slot: usize,
    ) {
        self.ensure_view(device, slot, members.len());
        let mut upload = vec![0u32; self.views[slot].members.size() as usize / 4];
        upload[..members.len()].copy_from_slice(members);
        queue.write_buffer(&self.views[slot].members, 0, bytemuck::cast_slice(&upload));
        let bindings = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("Game highlight mask bindings"),
            layout: &self.mask_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(visibility),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: self.views[slot].members.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::Sampler(&self.material_sampler),
                },
            ],
        });
        let mask = &self.mask.as_ref().expect("mask allocated").1;
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("Game highlight opaque coverage"),
                multiview_mask: None,
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: mask,
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
            pass.set_pipeline(&self.opaque);
            pass.set_bind_group(0, global, &[]);
            pass.set_bind_group(1, &bindings, &[]);
            pass.draw(0..3, 0..1);
        }
        if !transparent.is_empty() {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("Game highlight transparent coverage"),
                multiview_mask: None,
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: mask,
                    resolve_target: None,
                    depth_slice: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Load,
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: depth,
                    depth_ops: None,
                    stencil_ops: None,
                }),
                timestamp_writes: None,
                occlusion_query_set: None,
            });
            pass.set_pipeline(&self.transparent);
            pass.set_bind_group(0, global, &[]);
            pass.set_bind_group(1, &bindings, &[]);
            for draw in transparent {
                pass.draw(
                    0..draw.index_count,
                    draw.instance_index..draw.instance_index + 1,
                );
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub fn composite(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        output: &wgpu::TextureView,
        output_extent: [u32; 2],
        rect: Option<(u32, u32, u32, u32)>,
        style: HighlightStyle,
        grading: crate::pass::postprocess::Grading,
        frame: [u32; 2],
        source_uv_offset: [f32; 2],
        slot: usize,
    ) {
        let Some(style) = style.validated() else {
            return;
        };
        let Some((_, mask)) = &self.mask else { return };
        let size = rect.map_or(output_extent, |(_, _, w, h)| [w, h]);
        if size.contains(&0) {
            return;
        }
        let params = HighlightParams {
            color: style.color,
            inverse_output: [1.0 / size[0] as f32, 1.0 / size[1] as f32],
            width_pixels: style.width_pixels,
            show_mask: u32::from(style.show_mask) as f32,
            dream: grading.dream,
            frame: [frame[0] as f32, frame[1] as f32],
            time: grading.time,
            _pad: 0.0,
            source_uv_offset,
            _pad2: [0.0; 2],
        };
        queue.write_buffer(&self.views[slot].params, 0, bytemuck::bytes_of(&params));
        let bindings = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("Game highlight composite bindings"),
            layout: &self.composite_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(mask),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(&self.sampler),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: self.views[slot].params.as_entire_binding(),
                },
            ],
        });
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("Game highlight silhouette"),
            multiview_mask: None,
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: output,
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
        pass.set_pipeline(&self.composite);
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

    #[test]
    fn finite_style_and_uniform_layout() {
        assert_eq!(std::mem::size_of::<HighlightParams>(), 80);
        assert!(
            HighlightStyle {
                width_pixels: f32::NAN,
                ..Default::default()
            }
            .validated()
            .is_none()
        );
        assert!(
            HighlightStyle {
                color: [1.0, 1.0, 1.0, 0.0],
                ..Default::default()
            }
            .validated()
            .is_none()
        );
        assert_eq!(
            HighlightStyle {
                width_pixels: 100.0,
                ..Default::default()
            }
            .validated()
            .unwrap()
            .width_pixels,
            6.0
        );
    }

    #[test]
    fn selection_tracks_sorted_instances_and_transparent_tail() {
        use crate::command::{DrawCommand, SortKey};
        use crate::pass::taa::ReactiveDrawKey;
        let draw = |key, x| DrawCommand {
            sort_key: SortKey(key),
            vertex_offset: 12,
            index_offset: 24,
            index_count: 6,
            material_id: 2,
            transform: glam::Mat4::from_translation(glam::Vec3::X * x),
            casts_shadow: true,
            paint: 0,
        };
        let first = draw(10, 0.0);
        let other_instance = draw(1, 2.0);
        let glass = draw(5, 3.0);
        let selected = std::collections::HashSet::from([
            ReactiveDrawKey::of(&first),
            ReactiveDrawKey::of(&glass),
        ]);
        let mut opaque = vec![first.clone(), other_instance];
        opaque.sort_by_key(|d| d.sort_key);
        let (members, transparent) = frame_members(&opaque, 7, &[glass], &selected);
        assert_eq!(members, vec![0, 1]);
        assert_eq!(transparent[0].instance_index, 9);
        let moved = draw(10, 0.1);
        assert!(
            !selected.contains(&ReactiveDrawKey::of(&moved)),
            "a new frame must mark its current pose"
        );
        assert_eq!(
            frame_members(&opaque, 0, &[], &Default::default()).0,
            vec![0, 0]
        );
    }
}
