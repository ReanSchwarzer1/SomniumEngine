//! Screen-space velocity buffer (Phase 24AD).
//!
//! One full-screen pass over the depth buffer, producing UV-space motion for
//! every pixel. See `shaders/velocity.wgsl` for what it covers and what it does
//! not, and for the Wicked reference it follows.

/// Rg16Float: two signed UV offsets, and half precision is ample for a
/// quantity clamped to ±1 screen.
const VELOCITY_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rg16Float;

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct VelocityParams {
    inv_view_proj: [[f32; 4]; 4],
    prev_view_proj: [[f32; 4]; 4],
    inv_resolution: [f32; 2],
    valid: f32,
    _pad: f32,
}

/// How one opaque instance moved since last frame (C2, object and skinned motion vectors), indexed like the
/// instance buffer. `moving` 0: nothing of its own (the camera and wind cover it); 1: its last transform is
/// `prev_model`, and if `prev_vertex_base` is not `NO_POSE` its vertices were deformed too, last frame's pose starting
/// there in the previous-pose buffer.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, bytemuck::Pod, bytemuck::Zeroable)]
pub struct GpuMotion {
    pub prev_model: [[f32; 4]; 4],
    pub prev_vertex_base: u32,
    pub moving: u32,
    pub _pad: [u32; 2],
}

pub const NO_POSE: u32 = u32::MAX;

pub struct VelocityPass {
    pipeline: wgpu::RenderPipeline,
    layout: wgpu::BindGroupLayout,
    bind_group: wgpu::BindGroup,
    params: wgpu::Buffer,
    /// Per-instance motion, its capacity in records, and last frame's skinned poses (with their generation).
    motion: wgpu::Buffer,
    motion_capacity: usize,
    previous_pose: wgpu::Buffer,
    pose_generation: u64,
    depth_view: wgpu::TextureView,
    vis_view: wgpu::TextureView,
    texture: wgpu::Texture,
    view: wgpu::TextureView,
    prev_view_proj: glam::Mat4,
    /// False until a previous frame exists to reproject to.
    history_valid: bool,
}

impl VelocityPass {
    pub fn new(
        device: &wgpu::Device,
        shaders: &crate::shaders::Shaders,
        depth_view: &wgpu::TextureView,
        vis_view: &wgpu::TextureView,
        global_layout: &wgpu::BindGroupLayout,
        width: u32,
        height: u32,
    ) -> Self {
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("Velocity BGL"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Depth,
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                // The visibility buffer: which instance, so foliage wind can be
                // reconstructed (`wind.wgsl`).
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Uint,
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                // How each instance moved (`GpuMotion`), and last frame's skinned poses.
                storage_entry(3),
                storage_entry(4),
            ],
        });

        let params = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Velocity Params"),
            size: std::mem::size_of::<VelocityParams>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let (texture, view) = Self::alloc(device, width, height);
        let motion = motion_buffer(device, 1);
        let previous_pose = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("No previous pose"),
            size: 32,
            usage: wgpu::BufferUsages::STORAGE,
            mapped_at_creation: false,
        });
        let bind_group = Self::make_bind_group(device, &layout, depth_view, vis_view, &params, &motion, &previous_pose);

        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("Velocity Shader"),
            source: wgpu::ShaderSource::Wgsl(shaders.source_or_panic("velocity.wgsl").into()),
        });
        let pl = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("Velocity PL"),
            bind_group_layouts: &[Some(global_layout), Some(&layout)],
            immediate_size: 0,
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("Velocity Pipeline"),
            layout: Some(&pl),
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
                    format: VELOCITY_FORMAT,
                    blend: Some(wgpu::BlendState::REPLACE),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
                compilation_options: Default::default(),
            }),
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            cache: None,
        });

        Self {
            pipeline,
            layout,
            bind_group,
            params,
            motion,
            motion_capacity: 1,
            previous_pose,
            pose_generation: 0,
            depth_view: depth_view.clone(),
            vis_view: vis_view.clone(),
            texture,
            view,
            prev_view_proj: glam::Mat4::IDENTITY,
            history_valid: false,
        }
    }

    pub fn view(&self) -> &wgpu::TextureView {
        &self.view
    }

    pub fn texture(&self) -> &wgpu::Texture {
        &self.texture
    }

    fn alloc(device: &wgpu::Device, width: u32, height: u32) -> (wgpu::Texture, wgpu::TextureView) {
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("Velocity"),
            size: wgpu::Extent3d {
                width: width.max(1),
                height: height.max(1),
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: VELOCITY_FORMAT,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        (texture, view)
    }

    fn make_bind_group(
        device: &wgpu::Device,
        layout: &wgpu::BindGroupLayout,
        depth_view: &wgpu::TextureView,
        vis_view: &wgpu::TextureView,
        params: &wgpu::Buffer,
        motion: &wgpu::Buffer,
        previous_pose: &wgpu::Buffer,
    ) -> wgpu::BindGroup {
        device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("Velocity BG"),
            layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(depth_view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: params.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::TextureView(vis_view),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: motion.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 4,
                    resource: previous_pose.as_entire_binding(),
                },
            ],
        })
    }

    fn rebind(&mut self, device: &wgpu::Device) {
        self.bind_group = Self::make_bind_group(
            device,
            &self.layout,
            &self.depth_view,
            &self.vis_view,
            &self.params,
            &self.motion,
            &self.previous_pose,
        );
    }

    /// This frame's per-instance motion (indexed like the opaque instances) and the buffer of last frame's skinned
    /// poses with its generation (`AnimatedGeometry::previous_pose`).
    pub fn set_motion(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        motion: &[GpuMotion],
        previous_pose: Option<(&wgpu::Buffer, u64)>,
    ) {
        let mut rebind = false;
        if motion.len() > self.motion_capacity {
            self.motion_capacity = motion.len().next_power_of_two();
            self.motion = motion_buffer(device, self.motion_capacity);
            rebind = true;
        }
        if let Some((buffer, generation)) = previous_pose
            && generation != self.pose_generation
        {
            self.previous_pose = buffer.clone();
            self.pose_generation = generation;
            rebind = true;
        }
        if rebind {
            self.rebind(device);
        }
        if !motion.is_empty() {
            queue.write_buffer(&self.motion, 0, bytemuck::cast_slice(motion));
        }
    }

    pub fn resize(
        &mut self,
        device: &wgpu::Device,
        depth_view: &wgpu::TextureView,
        vis_view: &wgpu::TextureView,
        width: u32,
        height: u32,
    ) {
        let (texture, view) = Self::alloc(device, width, height);
        self.texture = texture;
        self.view = view;
        self.depth_view = depth_view.clone();
        self.vis_view = vis_view.clone();
        self.rebind(device);
        // The depth this reprojects from is a different image now.
        self.history_valid = false;
    }

    /// `view_proj_unjittered` must be un-jittered at both ends — see the
    /// shader's note and `TaaPass::record`, which measured what happens when it
    /// is not.
    pub fn record(
        &mut self,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        global: &wgpu::BindGroup,
        view_proj_unjittered: glam::Mat4,
        width: u32,
        height: u32,
    ) {
        queue.write_buffer(
            &self.params,
            0,
            bytemuck::bytes_of(&VelocityParams {
                inv_view_proj: view_proj_unjittered.inverse().to_cols_array_2d(),
                prev_view_proj: self.prev_view_proj.to_cols_array_2d(),
                inv_resolution: [1.0 / width as f32, 1.0 / height as f32],
                valid: f32::from(u8::from(self.history_valid)),
                _pad: 0.0,
            }),
        );

        let mut rpass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("Velocity Pass"),
            multiview_mask: None,
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: &self.view,
                resolve_target: None,
                depth_slice: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
        });
        rpass.set_pipeline(&self.pipeline);
        rpass.set_bind_group(0, global, &[]);
        rpass.set_bind_group(1, &self.bind_group, &[]);
        rpass.draw(0..3, 0..1);
        drop(rpass);

        self.prev_view_proj = view_proj_unjittered;
        self.history_valid = true;
    }
}

fn storage_entry(binding: u32) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::FRAGMENT,
        ty: wgpu::BindingType::Buffer {
            ty: wgpu::BufferBindingType::Storage { read_only: true },
            has_dynamic_offset: false,
            min_binding_size: None,
        },
        count: None,
    }
}

fn motion_buffer(device: &wgpu::Device, records: usize) -> wgpu::Buffer {
    device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("Instance motion"),
        size: (records.max(1) * std::mem::size_of::<GpuMotion>()) as u64,
        usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    })
}

#[cfg(test)]
mod tests {
    use super::{GpuMotion, VelocityParams};

    #[test]
    fn a_motion_record_is_eighty_bytes_as_the_shader_reads_it() {
        assert_eq!(std::mem::size_of::<GpuMotion>(), 80);
    }

    #[test]
    fn the_params_struct_is_a_multiple_of_sixteen() {
        // Uniform structs must be, and a trailing scalar pair is exactly where
        // that goes wrong — see `wgpu_api_gotchas.md`.
        assert_eq!(std::mem::size_of::<VelocityParams>() % 16, 0);
        assert_eq!(std::mem::size_of::<VelocityParams>(), 144);
    }
}
