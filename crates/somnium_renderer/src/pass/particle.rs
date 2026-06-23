//! Phase 11.5J: GPU Particle System render pass.
//!
//! Billboard instanced rendering (reference: bevy_enoki enoki2d crate):
//! - Each particle uploads 32 bytes (`GpuParticle`) to a storage buffer.
//! - `draw(0..6, 0..count)` emits 2 CCW triangles per instance.
//! - Vertex shader billboards corners in view space using camera_right/camera_up.
//! - Fragment shader applies a radial alpha soft-falloff.
//!
//! CPU simulation lives in `somnium_core::app::about_to_wait`; this pass only
//! uploads and renders what it receives each frame.

/// One particle instance uploaded to the GPU (32 bytes).
#[repr(C)]
#[derive(Copy, Clone, bytemuck::Pod, bytemuck::Zeroable)]
pub struct GpuParticle {
    /// World-space centre position.
    pub position: [f32; 3],
    /// Billboard half-width (metres).
    pub size:     f32,
    /// Linear RGBA color (faded by CPU simulation).
    pub color:    [f32; 4],
}

/// Per-frame view uniforms for the particle pass (96 bytes).
#[repr(C)]
#[derive(Copy, Clone, bytemuck::Pod, bytemuck::Zeroable)]
struct ParticleView {
    view_proj:    [f32; 16],  // offset  0, 64 bytes
    camera_right: [f32; 3],  // offset 64, 12 bytes
    _pad0:        f32,        // offset 76,  4 bytes
    camera_up:    [f32; 3],  // offset 80, 12 bytes
    _pad1:        f32,        // offset 92,  4 bytes
}

/// Maximum number of alive particles at once.
const MAX_PARTICLES: u64 = 10_000;

/// GPU billboard particle render pass.
pub struct ParticlePass {
    pipeline:        wgpu::RenderPipeline,
    view_buf:        wgpu::Buffer,
    instance_buf:    wgpu::Buffer,
    bind_group:      wgpu::BindGroup,
}

impl ParticlePass {
    pub fn new(device: &wgpu::Device, surface_format: wgpu::TextureFormat) -> Self {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label:  Some("Particle Shader"),
            source: wgpu::ShaderSource::Wgsl(
                include_str!("../shaders/particle.wgsl").into()
            ),
        });

        let view_buf = device.create_buffer(&wgpu::BufferDescriptor {
            label:              Some("Particle View Uniform"),
            size:               std::mem::size_of::<ParticleView>() as u64,
            usage:              wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let instance_buf = device.create_buffer(&wgpu::BufferDescriptor {
            label:              Some("Particle Instance Buffer"),
            size:               MAX_PARTICLES * std::mem::size_of::<GpuParticle>() as u64,
            usage:              wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label:   Some("Particle BGL"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding:    0,
                    visibility: wgpu::ShaderStages::VERTEX,
                    ty: wgpu::BindingType::Buffer {
                        ty:                 wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size:   None,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding:    1,
                    visibility: wgpu::ShaderStages::VERTEX,
                    ty: wgpu::BindingType::Buffer {
                        ty:                 wgpu::BufferBindingType::Storage { read_only: true },
                        has_dynamic_offset: false,
                        min_binding_size:   None,
                    },
                    count: None,
                },
            ],
        });

        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label:   Some("Particle BindGroup"),
            layout:  &bgl,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: view_buf.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 1, resource: instance_buf.as_entire_binding() },
            ],
        });

        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label:              Some("Particle Pipeline Layout"),
            bind_group_layouts: &[Some(&bgl)],
            immediate_size:     0,
        });

        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label:         Some("Particle Pipeline"),
            layout:        Some(&layout),
            multiview_mask: None,
            vertex: wgpu::VertexState {
                module:              &shader,
                entry_point:         Some("vs_main"),
                buffers:             &[],
                compilation_options: Default::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module:              &shader,
                entry_point:         Some("fs_main"),
                targets: &[Some(wgpu::ColorTargetState {
                    format:     surface_format,
                    blend:      Some(wgpu::BlendState::ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
                compilation_options: Default::default(),
            }),
            primitive: wgpu::PrimitiveState {
                topology:  wgpu::PrimitiveTopology::TriangleList,
                cull_mode: None,
                ..Default::default()
            },
            depth_stencil: None,
            multisample:   wgpu::MultisampleState::default(),
            cache:         None,
        });

        Self { pipeline, view_buf, instance_buf, bind_group }
    }

    /// Upload particle data and record a draw call.
    ///
    /// If `particles` is empty the pass does nothing.
    pub fn record(
        &self,
        queue:        &wgpu::Queue,
        encoder:      &mut wgpu::CommandEncoder,
        surface_view: &wgpu::TextureView,
        view_proj:    glam::Mat4,
        view_matrix:  glam::Mat4,
        particles:    &[GpuParticle],
    ) {
        if particles.is_empty() { return; }

        // Camera right = first column of view matrix (X axis in world, facing camera right).
        // Camera up    = second column.
        // (view = world→cam, so view.row(i) = i-th world-space basis vector of the camera)
        let camera_right = glam::Vec3::new(view_matrix.x_axis.x, view_matrix.y_axis.x, view_matrix.z_axis.x);
        let camera_up    = glam::Vec3::new(view_matrix.x_axis.y, view_matrix.y_axis.y, view_matrix.z_axis.y);

        let pview = ParticleView {
            view_proj:    view_proj.to_cols_array(),
            camera_right: camera_right.to_array(),
            _pad0:        0.0,
            camera_up:    camera_up.to_array(),
            _pad1:        0.0,
        };
        queue.write_buffer(&self.view_buf, 0, bytemuck::bytes_of(&pview));

        let count = particles.len().min(MAX_PARTICLES as usize);
        queue.write_buffer(&self.instance_buf, 0, bytemuck::cast_slice(&particles[..count]));

        let mut rpass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label:           Some("Particle Pass"),
            multiview_mask:  None,
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view:           surface_view,
                resolve_target: None,
                depth_slice:    None,
                ops: wgpu::Operations {
                    load:  wgpu::LoadOp::Load,
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: None,
            timestamp_writes:         None,
            occlusion_query_set:      None,
        });

        rpass.set_pipeline(&self.pipeline);
        rpass.set_bind_group(0, &self.bind_group, &[]);
        rpass.draw(0..6, 0..count as u32);
    }
}
