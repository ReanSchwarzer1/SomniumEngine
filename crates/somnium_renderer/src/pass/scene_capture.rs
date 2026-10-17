//! D2 (engine features record): a small, cheap view of the scene into a texture: what a pocket mirror, a monitor or
//! a screen shows.
//!
//! Not a second pass through the deferred renderer: a second view there is a whole second frame at the scene's size,
//! and its culling, history and froxels are the main camera's. This is a forward pass (`scene_capture.wgsl`) into its
//! own small target over the frame's opaque instances, with its own draw list (distance from its eye, not the main
//! camera's frustum: a mirror looks behind), lit by the sun, every lamp in reach and the environment, with fog, no
//! shadows. It renders into a private target and copies into the texture materials sample (`slot`), because that one
//! sits in the bindless array the pass itself binds. Shown the frame after it is drawn.
use crate::command::DrawCommand;
use glam::{Mat4, Vec3};

/// The texture's size, square. Small: it is a picture in the hand.
pub const SIZE: u32 = 384;
const FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba16Float;
/// How far past `reach` an instance's origin may be and still be drawn: big meshes' origins sit at a corner.
const ORIGIN_MARGIN: f32 = 40.0;

/// One view to capture this frame.
#[derive(Clone, Debug)]
pub struct CaptureRequest {
    pub view: Mat4,
    pub proj: Mat4,
    pub eye: Vec3,
    /// How far it sees, metres (instances further off, by their origin plus a margin, are not drawn).
    pub reach: f32,
    /// 0 indoors .. 1 outdoors: the sun has no shadows here to keep it out of a room.
    pub sun_scale: f32,
    pub ambient_scale: f32,
    /// Draws left out, by vertex offset: the viewer's own body, the thing doing the looking.
    pub hide: Vec<u32>,
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct Uniform {
    view_proj: [[f32; 4]; 4],
    inv_view_proj: [[f32; 4]; 4],
    eye: [f32; 3],
    sun_scale: f32,
    fog_density: f32,
    ambient_scale: f32,
    fog_asymmetry: f32,
    fog_height_falloff: f32,
    fog_base_height: f32,
    fog_sky_occlusion: f32,
    fog_distance: f32,
    _pad: f32,
}

pub struct SceneCapturePass {
    pipeline: wgpu::RenderPipeline,
    sky: wgpu::RenderPipeline,
    bind_group: wgpu::BindGroup,
    uniform: wgpu::Buffer,
    target: wgpu::Texture,
    target_view: wgpu::TextureView,
    depth_view: wgpu::TextureView,
    /// What materials sample, once registered in the bindless array (`slot`).
    shown: wgpu::Texture,
    pub slot: i32,
    args: wgpu::Buffer,
    args_capacity: usize,
    /// This frame's view, if anything asked for one.
    pub request: Option<CaptureRequest>,
    /// Instances drawn last time, for the profiler and tests.
    pub drawn: u32,
}

impl SceneCapturePass {
    pub fn new(
        device: &wgpu::Device,
        shaders: &crate::shaders::Shaders,
        global_layout: &wgpu::BindGroupLayout,
        env_view: &wgpu::TextureView,
        env_sampler: &wgpu::Sampler,
        atmosphere: &crate::pass::atmosphere::AtmospherePass,
    ) -> Self {
        let fragment = wgpu::ShaderStages::FRAGMENT;
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("Scene Capture BGL"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: fragment,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: fragment,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::Cube,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 3,
                    visibility: fragment,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
                // The atmosphere's transmittance and multiple-scattering tables: what lights the fog.
                lut_entry(4),
                lut_entry(5),
                wgpu::BindGroupLayoutEntry {
                    binding: 6,
                    visibility: fragment,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("Scene Capture Sampler"),
            address_mode_u: wgpu::AddressMode::Repeat,
            address_mode_v: wgpu::AddressMode::Repeat,
            address_mode_w: wgpu::AddressMode::Repeat,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::MipmapFilterMode::Linear,
            ..Default::default()
        });
        let uniform = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Scene Capture Uniform"),
            size: std::mem::size_of::<Uniform>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("Scene Capture BG"),
            layout: &layout,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: uniform.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::Sampler(&sampler) },
                wgpu::BindGroupEntry { binding: 2, resource: wgpu::BindingResource::TextureView(env_view) },
                wgpu::BindGroupEntry { binding: 3, resource: wgpu::BindingResource::Sampler(env_sampler) },
                wgpu::BindGroupEntry { binding: 4, resource: wgpu::BindingResource::TextureView(atmosphere.transmittance_view()) },
                wgpu::BindGroupEntry { binding: 5, resource: wgpu::BindingResource::TextureView(atmosphere.multiscatter_view()) },
                wgpu::BindGroupEntry { binding: 6, resource: wgpu::BindingResource::Sampler(atmosphere.sampler()) },
            ],
        });
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("Scene Capture Shader"),
            source: wgpu::ShaderSource::Wgsl(shaders.source_or_panic("scene_capture.wgsl").into()),
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("Scene Capture Pipeline Layout"),
            bind_group_layouts: &[Some(global_layout), Some(&layout)],
            immediate_size: 0,
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("Scene Capture Pipeline"),
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
                    format: FORMAT,
                    blend: None,
                    write_mask: wgpu::ColorWrites::ALL,
                })],
                compilation_options: Default::default(),
            }),
            // Both faces: the shader turns the normal toward the eye.
            primitive: wgpu::PrimitiveState { topology: wgpu::PrimitiveTopology::TriangleList, cull_mode: None, ..Default::default() },
            depth_stencil: Some(wgpu::DepthStencilState {
                format: wgpu::TextureFormat::Depth32Float,
                depth_write_enabled: Some(true),
                depth_compare: Some(wgpu::CompareFunction::Less),
                stencil: wgpu::StencilState::default(),
                bias: wgpu::DepthBiasState::default(),
            }),
            multisample: wgpu::MultisampleState::default(),
            cache: None,
        });
        // The sky: drawn first over the whole target at the far plane, everything else in front of it.
        let sky = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("Scene Capture Sky Pipeline"),
            layout: Some(&pipeline_layout),
            multiview_mask: None,
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_sky"),
                buffers: &[],
                compilation_options: Default::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_sky"),
                targets: &[Some(wgpu::ColorTargetState { format: FORMAT, blend: None, write_mask: wgpu::ColorWrites::ALL })],
                compilation_options: Default::default(),
            }),
            primitive: wgpu::PrimitiveState { topology: wgpu::PrimitiveTopology::TriangleList, cull_mode: None, ..Default::default() },
            depth_stencil: Some(wgpu::DepthStencilState {
                format: wgpu::TextureFormat::Depth32Float,
                depth_write_enabled: Some(false),
                depth_compare: Some(wgpu::CompareFunction::Always),
                stencil: wgpu::StencilState::default(),
                bias: wgpu::DepthBiasState::default(),
            }),
            multisample: wgpu::MultisampleState::default(),
            cache: None,
        });
        let texture = |label: &'static str, format: wgpu::TextureFormat, usage: wgpu::TextureUsages| {
            device.create_texture(&wgpu::TextureDescriptor {
                label: Some(label),
                size: wgpu::Extent3d { width: SIZE, height: SIZE, depth_or_array_layers: 1 },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format,
                usage,
                view_formats: &[],
            })
        };
        let target = texture("Scene Capture Target", FORMAT,
            wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC);
        let depth = texture("Scene Capture Depth", wgpu::TextureFormat::Depth32Float, wgpu::TextureUsages::RENDER_ATTACHMENT);
        let shown = texture("Scene Capture", FORMAT, wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST);
        Self {
            pipeline,
            sky,
            bind_group,
            uniform,
            target_view: target.create_view(&wgpu::TextureViewDescriptor::default()),
            target,
            depth_view: depth.create_view(&wgpu::TextureViewDescriptor::default()),
            shown,
            slot: -1,
            args: args_buffer(device, 1),
            args_capacity: 1,
            request: None,
            drawn: 0,
        }
    }

    /// The view materials sample, for registering in the bindless array.
    pub fn shown_view(&self) -> wgpu::TextureView {
        self.shown.create_view(&wgpu::TextureViewDescriptor::default())
    }

    /// Draw `request` over this frame's opaque `draws` (indexed like the instances), in the level's fog (`fog`, out to
    /// the distance given with it; None when the main view has none), and copy it to what is shown.
    #[allow(clippy::too_many_arguments)]
    pub fn record(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        global_bind_group: &wgpu::BindGroup,
        draws: &[DrawCommand],
        bounds: &dyn Fn(u32) -> Option<([f32; 3], [f32; 3])>,
        request: &CaptureRequest,
        fog: Option<(crate::pass::volumetric::FogSettings, f32)>,
        multi_draw: bool,
    ) {
        let args = draw_list(draws, bounds, request);
        let (fog, fog_distance) = fog.unwrap_or((crate::pass::volumetric::FogSettings { density: 0.0, ..Default::default() }, 0.0));
        self.drawn = args.len() as u32;
        let view_proj = request.proj * request.view;
        let uniform = Uniform {
            view_proj: view_proj.to_cols_array_2d(),
            inv_view_proj: view_proj.inverse().to_cols_array_2d(),
            eye: request.eye.to_array(),
            sun_scale: request.sun_scale,
            fog_density: fog.density.max(0.0),
            ambient_scale: request.ambient_scale,
            fog_asymmetry: fog.asymmetry,
            fog_height_falloff: fog.height_falloff.max(0.0),
            fog_base_height: fog.base_height,
            fog_sky_occlusion: fog.sky_occlusion,
            fog_distance,
            _pad: 0.0,
        };
        queue.write_buffer(&self.uniform, 0, bytemuck::bytes_of(&uniform));
        if multi_draw && !args.is_empty() {
            if args.len() > self.args_capacity {
                self.args_capacity = args.len().next_power_of_two();
                self.args = args_buffer(device, self.args_capacity);
            }
            queue.write_buffer(&self.args, 0, bytemuck::cast_slice(&args));
        }
        {
            let mut rpass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("Scene Capture Pass"),
                multiview_mask: None,
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &self.target_view,
                    resolve_target: None,
                    depth_slice: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &self.depth_view,
                    depth_ops: Some(wgpu::Operations { load: wgpu::LoadOp::Clear(1.0), store: wgpu::StoreOp::Discard }),
                    stencil_ops: None,
                }),
                timestamp_writes: None,
                occlusion_query_set: None,
            });
            rpass.set_bind_group(0, global_bind_group, &[]);
            rpass.set_bind_group(1, &self.bind_group, &[]);
            rpass.set_pipeline(&self.sky);
            rpass.draw(0..3, 0..1);
            rpass.set_pipeline(&self.pipeline);
            if multi_draw {
                if !args.is_empty() {
                    rpass.multi_draw_indirect(&self.args, 0, args.len() as u32);
                }
            } else {
                for [count, _, _, instance] in &args {
                    rpass.draw(0..*count, *instance..*instance + 1);
                }
            }
        }
        encoder.copy_texture_to_texture(
            self.target.as_image_copy(),
            self.shown.as_image_copy(),
            wgpu::Extent3d { width: SIZE, height: SIZE, depth_or_array_layers: 1 },
        );
    }
}

/// The draws a capture makes: every opaque instance in its view (its mesh's box against the capture's frustum, out to
/// its reach; one with no known box by its origin within reach plus a margin), except the hidden ones. Non-indexed
/// draws of the instance's index count (the vertex shader pulls its own indices).
fn draw_list(
    draws: &[DrawCommand],
    bounds: &dyn Fn(u32) -> Option<([f32; 3], [f32; 3])>,
    request: &CaptureRequest,
) -> Vec<[u32; 4]> {
    let far = request.reach + ORIGIN_MARGIN;
    let planes = crate::culling::frustum_planes(request.proj * request.view);
    draws
        .iter()
        .enumerate()
        .filter(|(_, d)| !request.hide.contains(&d.vertex_offset))
        .filter(|(_, d)| match bounds(d.vertex_offset) {
            Some((min, max)) => crate::culling::chunk_in_frustum(&planes, d.transform, min.into(), max.into()),
            None => d.transform.w_axis.truncate().distance(request.eye) < far,
        })
        .map(|(i, d)| [d.index_count, 1, 0, i as u32])
        .collect()
}

fn lut_entry(binding: u32) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::FRAGMENT,
        ty: wgpu::BindingType::Texture {
            sample_type: wgpu::TextureSampleType::Float { filterable: true },
            view_dimension: wgpu::TextureViewDimension::D2,
            multisampled: false,
        },
        count: None,
    }
}

fn args_buffer(device: &wgpu::Device, draws: usize) -> wgpu::Buffer {
    device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("Scene Capture Draws"),
        size: (draws.max(1) * 16) as u64,
        usage: wgpu::BufferUsages::INDIRECT | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn draw(vertex_offset: u32, at: Vec3) -> DrawCommand {
        DrawCommand {
            sort_key: crate::command::SortKey::new(0, 0, 0),
            vertex_offset,
            index_offset: 0,
            index_count: 36,
            material_id: 0,
            transform: Mat4::from_translation(at),
            casts_shadow: true,
            paint: 0,
        }
    }

    #[test]
    fn a_capture_draws_what_is_in_reach_behind_as_well_as_ahead_and_not_what_it_hides() {
        // No boxes known: by distance alone.
        let request = CaptureRequest {
            view: Mat4::IDENTITY,
            proj: Mat4::IDENTITY,
            eye: Vec3::ZERO,
            reach: 20.0,
            sun_scale: 1.0,
            ambient_scale: 1.0,
            hide: vec![7],
        };
        let draws = [draw(1, Vec3::new(0.0, 0.0, -5.0)), draw(2, Vec3::new(0.0, 0.0, 5.0)), draw(3, Vec3::new(0.0, 0.0, 500.0)),
            draw(7, Vec3::ZERO)];
        let list = draw_list(&draws, &|_| None, &request);
        assert_eq!(list, vec![[36, 1, 0, 0], [36, 1, 0, 1]], "ahead and behind, not 500 m off, not the hidden body");
    }

    #[test]
    fn a_capture_draws_only_what_its_view_holds() {
        // Looking down -z from the origin, a narrow view out to 40 m.
        let request = CaptureRequest {
            view: Mat4::IDENTITY,
            proj: Mat4::perspective_rh(0.3, 1.0, 0.1, 40.0),
            eye: Vec3::ZERO,
            reach: 40.0,
            sun_scale: 1.0,
            ambient_scale: 1.0,
            hide: vec![],
        };
        let unit = |_| Some(([-0.5_f32; 3], [0.5_f32; 3]));
        let draws = [draw(1, Vec3::new(0.0, 0.0, -10.0)), draw(2, Vec3::new(0.0, 0.0, 10.0)), draw(3, Vec3::new(8.0, 0.0, -10.0)),
            draw(4, Vec3::new(0.0, 0.0, -60.0))];
        assert_eq!(draw_list(&draws, &unit, &request), vec![[36, 1, 0, 0]], "ahead only: not behind, aside or past its reach");
    }

    #[test]
    fn the_uniform_is_the_size_the_shader_reads() {
        assert_eq!(std::mem::size_of::<Uniform>(), 176);
    }
}
