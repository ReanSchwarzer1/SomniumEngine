//! Device regression for close-up AO searches extending outside the viewport.
use somnium_renderer::shaders::Shaders;
use wgpu::util::DeviceExt;

#[path = "support/gpu.rs"]
mod gpu;

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct Params {
    projection: [[f32; 4]; 4],
    inverse: [[f32; 4]; 4],
    resolution: [f32; 2],
    radius: f32,
    power: f32,
    intensity: f32,
    frame: u32,
    near: f32,
    grain: u32,
}

#[test]
#[ignore = "Requires a graphics adapter; run for AO shader changes"]
fn close_open_surface_does_not_occlude_itself_at_screen_boundaries() {
    let (device, queue) = gpu::device();
    let size = 32;
    let depth = gpu::texture(
        &device,
        size,
        wgpu::TextureFormat::Depth32Float,
        wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::RENDER_ATTACHMENT,
    );
    let output = gpu::texture(
        &device,
        size,
        wgpu::TextureFormat::Rgba8Unorm,
        wgpu::TextureUsages::STORAGE_BINDING | wgpu::TextureUsages::COPY_SRC,
    );
    let grain = gpu::texture(
        &device,
        size,
        wgpu::TextureFormat::Rgba8Unorm,
        wgpu::TextureUsages::TEXTURE_BINDING,
    );
    let grain_view = grain.create_view(&wgpu::TextureViewDescriptor {
        dimension: Some(wgpu::TextureViewDimension::D2Array),
        ..Default::default()
    });
    let proj = glam::Mat4::perspective_rh(60.0_f32.to_radians(), 1.0, 0.1, 100.0);
    let uniform = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("Close AO fixture"),
        contents: bytemuck::bytes_of(&Params {
            projection: proj.to_cols_array_2d(),
            inverse: proj.inverse().to_cols_array_2d(),
            resolution: [1.0 / size as f32; 2],
            radius: 1.0,
            power: 2.0,
            intensity: 1.0,
            frame: 0,
            near: 0.1,
            grain: 0,
        }),
        usage: wgpu::BufferUsages::UNIFORM,
    });
    let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("Actual GTAO shader"),
        source: wgpu::ShaderSource::Wgsl(Shaders::new().source_or_panic("gtao.wgsl").into()),
    });
    let pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
        label: None,
        layout: None,
        module: &shader,
        entry_point: Some("main"),
        compilation_options: Default::default(),
        cache: None,
    });
    let depth_view = depth.create_view(&Default::default());
    let output_view = output.create_view(&Default::default());
    let bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: None,
        layout: &pipeline.get_bind_group_layout(0),
        entries: &[
            gpu::view(0, &depth_view),
            gpu::view(1, &output_view),
            wgpu::BindGroupEntry {
                binding: 2,
                resource: uniform.as_entire_binding(),
            },
            gpu::view(5, &grain_view),
        ],
    });
    for distance in [0.18, 0.4, 2.0] {
        let clip = proj * glam::Vec4::new(0.0, 0.0, -distance, 1.0);
        let mut encoder = device.create_command_encoder(&Default::default());
        {
            let _pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("Unoccluded flat plane"),
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &depth_view,
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Clear(clip.z / clip.w),
                        store: wgpu::StoreOp::Store,
                    }),
                    stencil_ops: None,
                }),
                ..Default::default()
            });
        }
        {
            let mut pass = encoder.begin_compute_pass(&Default::default());
            pass.set_pipeline(&pipeline);
            pass.set_bind_group(0, &bind, &[]);
            pass.dispatch_workgroups(size.div_ceil(8), size.div_ceil(8), 1);
        }
        let bytes = gpu::read_rgba(&device, &queue, encoder, &output);
        let minimum = bytes.chunks_exact(4).map(|pixel| pixel[3]).min().unwrap();
        println!("plane {distance}m: minimum visibility {minimum}/255");
        assert!(
            minimum >= 230,
            "An open surface became dark at {distance}m: {minimum}"
        );
    }
}
