//! The preview studio: a few draws, lit and tone-mapped into a small square target of their own, for what the editor
//! shows beside the level rather than in it. The Materials workspace's turning sphere, cube or plane is one user; an
//! asset's 3-D thumbnail is another.
//!
//! Not a second view through the deferred renderer, which is a whole frame at the viewport's size with the main
//! camera's history. A forward pass (`preview_studio.wgsl`) over draws pulled from the global geometry pool, with each
//! material read from the bindless material buffer, so a preview cannot disagree with the level about what a material
//! is: an edit that reaches the GPU material reaches the preview in the same frame. It shades with `brdf.wgsl`'s
//! `evaluate_brdf` (the deferred shading's BRDF, with this frame's energy terms) under a fixed three-light rig turned
//! with the camera, plus the engine's prefiltered environment with its brightness normalised. No shadows or fog.
//!
//! Each target is 4x multisampled and resolved into an 8-bit texture that holds sRGB-encoded bytes: what the UI's
//! atlases hold, so the editor can bind it as one of its own textures (`studio_view`) and draw it like a thumbnail.
//! Targets are independent (their own uniform and draw list), so several can render in one frame.
use glam::{Mat4, Vec3};
use somnium_asset::Vertex;

/// What the UI binds: the resolved texels, read as sRGB-encoded bytes.
const SHOWN_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;
/// What the passes write through, so the hardware encodes (and resolves) in linear light.
const RENDER_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8UnormSrgb;
const DEPTH_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Depth32Float;
const SAMPLES: u32 = 4;
/// `Instance._padding` bit the shader reads: the draw's material is alpha-blended.
const DRAW_BLENDED: u32 = 1;

/// The built-in shapes a material is shown on.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum StudioShape {
    #[default]
    Sphere,
    Cube,
    Plane,
}

impl StudioShape {
    pub const ALL: [Self; 3] = [Self::Sphere, Self::Cube, Self::Plane];

    #[must_use]
    pub fn index(self) -> usize {
        self as usize
    }

    #[must_use]
    pub fn from_index(index: u8) -> Option<Self> {
        Self::ALL.get(usize::from(index)).copied()
    }
}

/// One draw: a mesh span in the global pool, its material slot and where it stands.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct StudioDraw {
    pub vertex_offset: u32,
    pub index_offset: u32,
    pub index_count: u32,
    pub material_id: u32,
    pub model: Mat4,
}

/// What to draw into a target this frame, and from where.
#[derive(Clone, Debug)]
pub struct StudioRequest {
    pub draws: Vec<StudioDraw>,
    pub view: Mat4,
    pub proj: Mat4,
    pub eye: Vec3,
    /// Linear colour behind the subject: the panel it is shown in.
    pub backdrop: [f32; 3],
}

/// A studio target, from [`crate::SomniumRenderer::studio_target`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct StudioTargetId(pub u32);

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct Uniform {
    view_proj: [[f32; 4]; 4],
    inv_view_proj: [[f32; 4]; 4],
    eye: [f32; 3],
    exposure: f32,
    backdrop: [f32; 3],
    env_level: f32,
    light_dir: [[f32; 4]; 3],
    light_color: [[f32; 4]; 3],
}

/// Exposure before the tone map.
const EXPOSURE: f32 = 0.9;
/// Mean luminance the environment is scaled to.
const ENV_LEVEL: f32 = 0.3;

/// The studio's lights for a camera: key (warm, above and to the left), fill (cool, low right) and rim (behind),
/// each as (direction toward the light, linear colour). Fixed to the camera, so turning the subject shows it under
/// the same light from every side.
#[must_use]
pub fn light_rig(view: Mat4) -> [(Vec3, Vec3); 3] {
    let camera = view.inverse();
    let right = camera.x_axis.truncate().normalize_or_zero();
    let up = camera.y_axis.truncate().normalize_or_zero();
    // A right-handed view looks down its own -z, so +z points back at the camera.
    let back = camera.z_axis.truncate().normalize_or_zero();
    [
        ((-0.55 * right + 0.65 * up + 0.55 * back).normalize(), Vec3::new(1.0, 0.95, 0.88) * 2.6),
        ((0.75 * right - 0.1 * up + 0.6 * back).normalize(), Vec3::new(0.6, 0.7, 0.85) * 0.7),
        ((0.45 * right + 0.5 * up - 0.75 * back).normalize(), Vec3::splat(1.8)),
    ]
}

/// A camera that holds the box `min..max` whole, looking at its centre from `yaw` around and `pitch` above
/// (radians), with a vertical field of view `fov_y`, for a square target. Returns (view, proj, eye).
#[must_use]
pub fn frame_bounds(min: Vec3, max: Vec3, yaw: f32, pitch: f32, fov_y: f32) -> (Mat4, Mat4, Vec3) {
    let centre = (min + max) * 0.5;
    let radius = ((max - min) * 0.5).length().max(1.0e-3);
    let distance = radius / (fov_y * 0.5).sin() * 1.04;
    let direction = Vec3::new(yaw.sin() * pitch.cos(), pitch.sin(), yaw.cos() * pitch.cos());
    let eye = centre + direction * distance;
    let view = Mat4::look_at_rh(eye, centre, Vec3::Y);
    let near = (distance - radius * 1.5).max(distance * 0.01);
    let proj = Mat4::perspective_rh(fov_y, 1.0, near, distance + radius * 1.5);
    (view, proj, eye)
}

/// Where a built-in shape stands at turntable angle `yaw` (radians): the sphere and the cube turn about the vertical;
/// the plane, tilted back toward the camera, sways a little rather than turning edge-on.
#[must_use]
pub fn turntable(shape: StudioShape, yaw: f32) -> Mat4 {
    match shape {
        StudioShape::Sphere => Mat4::from_rotation_y(yaw),
        StudioShape::Cube => Mat4::from_rotation_y(yaw + std::f32::consts::FRAC_PI_4),
        StudioShape::Plane => Mat4::from_rotation_x(-0.55) * Mat4::from_rotation_y(0.45 * yaw.sin()),
    }
}

/// The camera for the built-in shapes: a little above, framing a unit sphere.
#[must_use]
pub fn shape_camera() -> (Mat4, Mat4, Vec3) {
    frame_bounds(Vec3::splat(-1.0), Vec3::splat(1.0), 0.0, 0.3, 0.5)
}

/// The built-in shapes' meshes, counter-clockwise outward (front faces face out), unit-sized.
#[must_use]
pub fn shape_mesh(shape: StudioShape) -> (Vec<Vertex>, Vec<u32>) {
    match shape {
        StudioShape::Sphere => sphere_mesh(96, 48, 1.0),
        StudioShape::Cube => cube_mesh(0.72),
        StudioShape::Plane => plane_mesh(1.25),
    }
}

/// Latitude-longitude sphere. The texture goes twice around, as the CPU preview's sphere wears it, so a tiling
/// texture reads at about the density it has on a metre-scale prop.
fn sphere_mesh(segments: u32, rings: u32, radius: f32) -> (Vec<Vertex>, Vec<u32>) {
    let mut vertices = Vec::with_capacity(((segments + 1) * (rings + 1)) as usize);
    for ring in 0..=rings {
        let v = ring as f32 / rings as f32;
        let theta = v * std::f32::consts::PI;
        for segment in 0..=segments {
            let u = segment as f32 / segments as f32;
            let phi = u * std::f32::consts::TAU;
            let normal = Vec3::new(theta.sin() * phi.sin(), theta.cos(), theta.sin() * phi.cos());
            vertices.push(Vertex { position: (normal * radius).to_array(), normal: normal.to_array(), uv: [u * 2.0, v] });
        }
    }
    let row = segments + 1;
    let mut indices = Vec::with_capacity((segments * rings * 6) as usize);
    for ring in 0..rings {
        for segment in 0..segments {
            let a = ring * row + segment;
            let b = a + row;
            indices.extend_from_slice(&[a, b, a + 1, a + 1, b, b + 1]);
        }
    }
    (vertices, indices)
}

/// A cube of half-extent `half`, each face its own 0..1 square of texture.
fn cube_mesh(half: f32) -> (Vec<Vertex>, Vec<u32>) {
    let mut vertices = Vec::with_capacity(24);
    let mut indices = Vec::with_capacity(36);
    for normal in [Vec3::X, Vec3::NEG_X, Vec3::Y, Vec3::NEG_Y, Vec3::Z, Vec3::NEG_Z] {
        // Two axes across the face, chosen so right x up = the outward normal.
        let up = if normal.y.abs() > 0.5 { Vec3::NEG_Z * normal.y.signum() } else { Vec3::Y };
        let right = up.cross(normal);
        let base = vertices.len() as u32;
        for (x, y) in [(-1.0, -1.0), (1.0, -1.0), (1.0, 1.0), (-1.0, 1.0)] {
            let position = (normal + right * x + up * y) * half;
            vertices.push(Vertex {
                position: position.to_array(),
                normal: normal.to_array(),
                uv: [(x + 1.0) * 0.5, (1.0 - y) * 0.5],
            });
        }
        indices.extend_from_slice(&[base, base + 1, base + 2, base, base + 2, base + 3]);
    }
    (vertices, indices)
}

/// A square in the XY plane facing +z, `half` from its centre to each side.
fn plane_mesh(half: f32) -> (Vec<Vertex>, Vec<u32>) {
    let vertices = [(-1.0, -1.0), (1.0, -1.0), (1.0, 1.0), (-1.0, 1.0)]
        .into_iter()
        .map(|(x, y)| Vertex { position: [x * half, y * half, 0.0], normal: [0.0, 0.0, 1.0], uv: [(x + 1.0) * 0.5, (1.0 - y) * 0.5] })
        .collect();
    (vertices, vec![0, 1, 2, 0, 2, 3])
}

/// A built-in shape's place in the global pool. Reserved spans, so a scene switch's release never takes them.
#[derive(Clone, Copy, Debug)]
struct ShapeSpan {
    vertex_offset: u32,
    index_offset: u32,
    index_count: u32,
}

struct StudioTarget {
    size: u32,
    msaa_view: wgpu::TextureView,
    depth_view: wgpu::TextureView,
    shown: wgpu::Texture,
    resolve_view: wgpu::TextureView,
    uniform: wgpu::Buffer,
    draws: wgpu::Buffer,
    draws_capacity: usize,
    bind_group: wgpu::BindGroup,
}

pub struct PreviewStudio {
    opaque: wgpu::RenderPipeline,
    /// Blended draws go in twice: back faces, then front faces, so a glass ball shows its far side through its near.
    blend_back: wgpu::RenderPipeline,
    blend_front: wgpu::RenderPipeline,
    backdrop: wgpu::RenderPipeline,
    layout: wgpu::BindGroupLayout,
    sampler: wgpu::Sampler,
    env_view: wgpu::TextureView,
    env_sampler: wgpu::Sampler,
    targets: Vec<StudioTarget>,
    pending: Vec<(StudioTargetId, StudioRequest)>,
    shapes: Option<[ShapeSpan; 3]>,
    /// The geometry pool had no room for the shapes; not retried.
    shapes_refused: bool,
    /// Requests recorded last frame, for tests and the profiler.
    pub recorded: u32,
}

impl PreviewStudio {
    pub fn new(
        device: &wgpu::Device,
        shaders: &crate::shaders::Shaders,
        global_layout: &wgpu::BindGroupLayout,
        env_view: &wgpu::TextureView,
        env_sampler: &wgpu::Sampler,
    ) -> Self {
        let both = wgpu::ShaderStages::VERTEX_FRAGMENT;
        let entry = |binding, visibility, ty| wgpu::BindGroupLayoutEntry { binding, visibility, ty, count: None };
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("Preview Studio BGL"),
            entries: &[
                entry(0, both, wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                }),
                entry(1, wgpu::ShaderStages::VERTEX, wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Storage { read_only: true },
                    has_dynamic_offset: false,
                    min_binding_size: None,
                }),
                entry(2, wgpu::ShaderStages::FRAGMENT, wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering)),
                // The environment is read in the vertex stage too: its brightness, once per vertex.
                entry(3, both, wgpu::BindingType::Texture {
                    sample_type: wgpu::TextureSampleType::Float { filterable: true },
                    view_dimension: wgpu::TextureViewDimension::Cube,
                    multisampled: false,
                }),
                entry(4, both, wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering)),
            ],
        });
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("Preview Studio Sampler"),
            address_mode_u: wgpu::AddressMode::Repeat,
            address_mode_v: wgpu::AddressMode::Repeat,
            address_mode_w: wgpu::AddressMode::Repeat,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::MipmapFilterMode::Linear,
            anisotropy_clamp: 8,
            ..Default::default()
        });
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("Preview Studio Shader"),
            source: wgpu::ShaderSource::Wgsl(shaders.source_or_panic("preview_studio.wgsl").into()),
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("Preview Studio Pipeline Layout"),
            bind_group_layouts: &[Some(global_layout), Some(&layout)],
            immediate_size: 0,
        });
        let pipeline = |label, vs, fs, cull: Option<wgpu::Face>, blend: Option<wgpu::BlendState>, depth_write: bool, compare| {
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(label),
                layout: Some(&pipeline_layout),
                multiview_mask: None,
                vertex: wgpu::VertexState {
                    module: &shader,
                    entry_point: Some(vs),
                    buffers: &[],
                    compilation_options: Default::default(),
                },
                fragment: Some(wgpu::FragmentState {
                    module: &shader,
                    entry_point: Some(fs),
                    targets: &[Some(wgpu::ColorTargetState { format: RENDER_FORMAT, blend, write_mask: wgpu::ColorWrites::ALL })],
                    compilation_options: Default::default(),
                }),
                primitive: wgpu::PrimitiveState {
                    topology: wgpu::PrimitiveTopology::TriangleList,
                    front_face: wgpu::FrontFace::Ccw,
                    cull_mode: cull,
                    ..Default::default()
                },
                depth_stencil: Some(wgpu::DepthStencilState {
                    format: DEPTH_FORMAT,
                    depth_write_enabled: Some(depth_write),
                    depth_compare: Some(compare),
                    stencil: wgpu::StencilState::default(),
                    bias: wgpu::DepthBiasState::default(),
                }),
                multisample: wgpu::MultisampleState { count: SAMPLES, mask: !0, alpha_to_coverage_enabled: false },
                cache: None,
            })
        };
        let less = wgpu::CompareFunction::Less;
        let premultiplied = Some(wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING);
        // Opaque draws keep both faces (the shader turns the normal to the eye): a plane, an open mesh.
        let opaque = pipeline("Preview Studio Opaque", "vs_main", "fs_main", None, None, true, less);
        let blend_back = pipeline("Preview Studio Blend Back", "vs_main", "fs_main", Some(wgpu::Face::Front), premultiplied, false, less);
        let blend_front = pipeline("Preview Studio Blend Front", "vs_main", "fs_main", Some(wgpu::Face::Back), premultiplied, false, less);
        let backdrop = pipeline("Preview Studio Backdrop", "vs_backdrop", "fs_backdrop", None, None, false, wgpu::CompareFunction::Always);
        Self {
            opaque,
            blend_back,
            blend_front,
            backdrop,
            layout,
            sampler,
            env_view: env_view.clone(),
            env_sampler: env_sampler.clone(),
            targets: Vec::new(),
            pending: Vec::new(),
            shapes: None,
            shapes_refused: false,
            recorded: 0,
        }
    }

    /// A new square target `size` texels on a side.
    pub fn create_target(&mut self, device: &wgpu::Device, size: u32) -> StudioTargetId {
        let size = size.clamp(16, 2048);
        let texture = |label, format, samples, usage, view_formats: &[wgpu::TextureFormat]| {
            device.create_texture(&wgpu::TextureDescriptor {
                label: Some(label),
                size: wgpu::Extent3d { width: size, height: size, depth_or_array_layers: 1 },
                mip_level_count: 1,
                sample_count: samples,
                dimension: wgpu::TextureDimension::D2,
                format,
                usage,
                view_formats,
            })
        };
        let attachment = wgpu::TextureUsages::RENDER_ATTACHMENT;
        let msaa = texture("Preview Studio MSAA", RENDER_FORMAT, SAMPLES, attachment, &[]);
        let depth = texture("Preview Studio Depth", DEPTH_FORMAT, SAMPLES, attachment, &[]);
        let shown = texture(
            "Preview Studio",
            SHOWN_FORMAT,
            1,
            attachment | wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_SRC,
            &[RENDER_FORMAT],
        );
        let resolve_view = shown.create_view(&wgpu::TextureViewDescriptor { format: Some(RENDER_FORMAT), ..Default::default() });
        let uniform = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Preview Studio Uniform"),
            size: std::mem::size_of::<Uniform>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let draws = draws_buffer(device, 4);
        let bind_group = self.bind_group(device, &uniform, &draws);
        self.targets.push(StudioTarget {
            size,
            msaa_view: msaa.create_view(&wgpu::TextureViewDescriptor::default()),
            depth_view: depth.create_view(&wgpu::TextureViewDescriptor::default()),
            shown,
            resolve_view,
            uniform,
            draws,
            draws_capacity: 4,
            bind_group,
        });
        StudioTargetId(self.targets.len() as u32 - 1)
    }

    fn bind_group(&self, device: &wgpu::Device, uniform: &wgpu::Buffer, draws: &wgpu::Buffer) -> wgpu::BindGroup {
        device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("Preview Studio BG"),
            layout: &self.layout,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: uniform.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 1, resource: draws.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 2, resource: wgpu::BindingResource::Sampler(&self.sampler) },
                wgpu::BindGroupEntry { binding: 3, resource: wgpu::BindingResource::TextureView(&self.env_view) },
                wgpu::BindGroupEntry { binding: 4, resource: wgpu::BindingResource::Sampler(&self.env_sampler) },
            ],
        })
    }

    /// The resolved picture, as the UI binds it (8-bit, sRGB-encoded bytes).
    #[must_use]
    pub fn view(&self, id: StudioTargetId) -> Option<wgpu::TextureView> {
        Some(self.targets.get(id.0 as usize)?.shown.create_view(&wgpu::TextureViewDescriptor::default()))
    }

    /// The resolved texture itself, for a copy out (a thumbnail's readback).
    #[must_use]
    pub fn texture(&self, id: StudioTargetId) -> Option<&wgpu::Texture> {
        Some(&self.targets.get(id.0 as usize)?.shown)
    }

    #[must_use]
    pub fn size(&self, id: StudioTargetId) -> Option<u32> {
        Some(self.targets.get(id.0 as usize)?.size)
    }

    /// Draw `request` into `id` this frame, in place of anything asked of it earlier in the frame.
    pub fn queue(&mut self, id: StudioTargetId, request: StudioRequest) {
        self.pending.retain(|(queued, _)| *queued != id);
        self.pending.push((id, request));
    }

    /// Whether anything is waiting to be drawn.
    #[must_use]
    pub fn has_pending(&self) -> bool {
        !self.pending.is_empty()
    }

    /// A built-in shape as a draw of `material_id` standing at `model`, uploading the shapes on first use.
    pub fn shape_draw(
        &mut self,
        geometry: &mut crate::geometry::GeometryPool,
        queue: &wgpu::Queue,
        shape: StudioShape,
        material_id: u32,
        model: Mat4,
    ) -> Option<StudioDraw> {
        if self.shapes.is_none() && !self.shapes_refused {
            let mut spans = Vec::with_capacity(3);
            for shape in StudioShape::ALL {
                let (vertices, indices) = shape_mesh(shape);
                let vertex_offset = geometry.reserve_vertices(vertices.len() as u32);
                let index_offset = geometry.reserve_indices(indices.len() as u32);
                let (Some(vertex_offset), Some(index_offset)) = (vertex_offset, index_offset) else {
                    // A full pool: hand back what was taken, and do not ask (and log) again every frame.
                    for span in spans.iter().map(|s: &ShapeSpan| (s.vertex_offset, s.index_offset)).chain([(
                        vertex_offset.unwrap_or(u32::MAX),
                        index_offset.unwrap_or(u32::MAX),
                    )]) {
                        geometry.release_vertices(span.0);
                        geometry.release_indices(span.1);
                    }
                    self.shapes_refused = true;
                    return None;
                };
                geometry.write_vertices(queue, vertex_offset, &vertices);
                geometry.write_indices(queue, index_offset, &indices);
                spans.push(ShapeSpan { vertex_offset, index_offset, index_count: indices.len() as u32 });
            }
            self.shapes = Some([spans[0], spans[1], spans[2]]);
        }
        let span = self.shapes?[shape.index()];
        Some(StudioDraw {
            vertex_offset: span.vertex_offset,
            index_offset: span.index_offset,
            index_count: span.index_count,
            material_id,
            model,
        })
    }

    /// Record every queued request. `blended` says whether a material slot is alpha-blended.
    pub fn record(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        global_bind_group: &wgpu::BindGroup,
        blended: &dyn Fn(u32) -> bool,
    ) {
        let pending = std::mem::take(&mut self.pending);
        self.recorded = 0;
        for (id, request) in pending {
            let index = id.0 as usize;
            if index >= self.targets.len() {
                continue;
            }
            let gpu: Vec<crate::instance::GpuInstanceData> = request
                .draws
                .iter()
                .map(|draw| instance(draw, blended(draw.material_id)))
                .collect();
            if gpu.len() > self.targets[index].draws_capacity {
                let capacity = gpu.len().next_power_of_two();
                let draws = draws_buffer(device, capacity);
                let bind_group = self.bind_group(device, &self.targets[index].uniform, &draws);
                let target = &mut self.targets[index];
                target.draws = draws;
                target.draws_capacity = capacity;
                target.bind_group = bind_group;
            }
            let target = &self.targets[index];
            queue.write_buffer(&target.uniform, 0, bytemuck::bytes_of(&uniform(&request)));
            if !gpu.is_empty() {
                queue.write_buffer(&target.draws, 0, bytemuck::cast_slice(&gpu));
            }
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("Preview Studio Pass"),
                multiview_mask: None,
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &target.msaa_view,
                    resolve_target: Some(&target.resolve_view),
                    depth_slice: None,
                    ops: wgpu::Operations { load: wgpu::LoadOp::Clear(wgpu::Color::BLACK), store: wgpu::StoreOp::Discard },
                })],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &target.depth_view,
                    depth_ops: Some(wgpu::Operations { load: wgpu::LoadOp::Clear(1.0), store: wgpu::StoreOp::Discard }),
                    stencil_ops: None,
                }),
                timestamp_writes: None,
                occlusion_query_set: None,
            });
            pass.set_bind_group(0, global_bind_group, &[]);
            pass.set_bind_group(1, &target.bind_group, &[]);
            pass.set_pipeline(&self.backdrop);
            pass.draw(0..3, 0..1);
            let draws = || request.draws.iter().enumerate();
            pass.set_pipeline(&self.opaque);
            for (i, draw) in draws().filter(|(_, d)| !blended(d.material_id)) {
                pass.draw(0..draw.index_count, i as u32..i as u32 + 1);
            }
            for pipeline in [&self.blend_back, &self.blend_front] {
                pass.set_pipeline(pipeline);
                for (i, draw) in draws().filter(|(_, d)| blended(d.material_id)) {
                    pass.draw(0..draw.index_count, i as u32..i as u32 + 1);
                }
            }
            self.recorded += 1;
        }
    }
}

/// The GPU form of a draw: the pool's own instance record, with the blend bit in its spare word.
fn instance(draw: &StudioDraw, blended: bool) -> crate::instance::GpuInstanceData {
    crate::instance::GpuInstanceData {
        model_matrix: draw.model.to_cols_array_2d(),
        material_id: draw.material_id,
        mesh_vertex_offset: draw.vertex_offset,
        mesh_index_offset: draw.index_offset,
        _padding: if blended { DRAW_BLENDED } else { 0 },
    }
}

fn uniform(request: &StudioRequest) -> Uniform {
    let view_proj = request.proj * request.view;
    let rig = light_rig(request.view);
    Uniform {
        view_proj: view_proj.to_cols_array_2d(),
        inv_view_proj: view_proj.inverse().to_cols_array_2d(),
        eye: request.eye.to_array(),
        exposure: EXPOSURE,
        backdrop: request.backdrop,
        env_level: ENV_LEVEL,
        light_dir: rig.map(|(dir, _)| dir.extend(0.0).to_array()),
        light_color: rig.map(|(_, color)| color.extend(0.0).to_array()),
    }
}

fn draws_buffer(device: &wgpu::Device, draws: usize) -> wgpu::Buffer {
    device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("Preview Studio Draws"),
        size: (draws.max(1) * std::mem::size_of::<crate::instance::GpuInstanceData>()) as u64,
        usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_uniform_is_the_size_the_shader_reads() {
        // view_proj 64, inv 64, eye+exposure 16, backdrop+env_level 16, three light directions 48, three colours 48.
        assert_eq!(std::mem::size_of::<Uniform>(), 256);
        assert_eq!(std::mem::size_of::<crate::instance::GpuInstanceData>(), 80, "the WGSL Instance the draws reuse");
    }

    /// Front faces face out (the blended pipelines cull by winding), normals are unit and agree with the winding,
    /// every index is in range and every UV is finite.
    #[test]
    fn the_shapes_are_wound_outward_with_unit_normals() {
        for shape in StudioShape::ALL {
            let (vertices, indices) = shape_mesh(shape);
            assert!(!indices.is_empty() && indices.len() % 3 == 0, "{shape:?}");
            assert!(indices.iter().all(|&i| (i as usize) < vertices.len()), "{shape:?} index out of range");
            for v in &vertices {
                assert!((Vec3::from(v.normal).length() - 1.0).abs() < 1.0e-4, "{shape:?} normal not unit");
                assert!(v.uv.iter().all(|c| c.is_finite()));
            }
            let mut agreeing = 0;
            let mut total = 0;
            for tri in indices.chunks_exact(3) {
                let [a, b, c] = [tri[0], tri[1], tri[2]].map(|i| Vec3::from(vertices[i as usize].position));
                let face = (b - a).cross(c - a);
                if face.length() < 1.0e-9 {
                    continue; // the sphere's poles
                }
                total += 1;
                let normal = Vec3::from(vertices[tri[0] as usize].normal);
                agreeing += usize::from(face.dot(normal) > 0.0);
            }
            assert_eq!(agreeing, total, "{shape:?}: {agreeing} of {total} triangles wound outward");
        }
    }

    #[test]
    fn the_camera_holds_the_whole_box_in_view() {
        let (min, max) = (Vec3::new(-2.0, 0.0, -1.0), Vec3::new(4.0, 3.0, 1.0));
        for yaw in [0.0_f32, 1.0, 2.5, -2.0] {
            let (view, proj, eye) = frame_bounds(min, max, yaw, 0.35, 0.6);
            let clip = proj * view;
            for corner in 0..8 {
                let p = Vec3::new(
                    if corner & 1 == 0 { min.x } else { max.x },
                    if corner & 2 == 0 { min.y } else { max.y },
                    if corner & 4 == 0 { min.z } else { max.z },
                );
                let ndc = clip.project_point3(p);
                assert!(ndc.x.abs() <= 1.0 && ndc.y.abs() <= 1.0, "corner {p} leaves the frame at yaw {yaw}: {ndc}");
                assert!((0.0..=1.0).contains(&ndc.z), "corner {p} is clipped by depth: {ndc}");
            }
            assert!(eye.y > (min.y + max.y) * 0.5, "the camera looks down a little");
        }
    }

    #[test]
    fn the_rig_keeps_its_key_light_above_and_in_front_whatever_the_camera() {
        for yaw in [0.0_f32, 1.3, 3.0] {
            let (view, _, eye) = frame_bounds(Vec3::splat(-1.0), Vec3::splat(1.0), yaw, 0.3, 0.5);
            let [(key, key_color), (_, fill_color), (rim, _)] = light_rig(view);
            let toward_camera = eye.normalize();
            assert!(key.y > 0.3 && key.dot(toward_camera) > 0.2, "key light above and in front: {key}");
            assert!(rim.dot(toward_camera) < -0.3, "rim light behind: {rim}");
            assert!(key_color.x > fill_color.x, "the key is the strongest light");
        }
    }

    #[test]
    fn the_turntable_turns_the_sphere_and_sways_the_plane() {
        let p = Vec3::new(0.0, 0.0, 1.0);
        let turned = turntable(StudioShape::Sphere, std::f32::consts::FRAC_PI_2).transform_point3(p);
        assert!((turned - Vec3::X).length() < 1.0e-5);
        // The plane never turns edge-on to a camera in front of it.
        for step in 0..64 {
            let normal = turntable(StudioShape::Plane, step as f32 * 0.2).transform_vector3(Vec3::Z);
            assert!(normal.z > 0.6, "plane edge-on at step {step}: {normal}");
        }
        assert_eq!(StudioShape::from_index(2), Some(StudioShape::Plane));
        assert_eq!(StudioShape::from_index(3), None);
    }
}
