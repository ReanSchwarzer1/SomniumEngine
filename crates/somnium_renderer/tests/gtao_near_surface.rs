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
        wgpu::TextureUsages::STORAGE_BINDING | wgpu::TextureUsages::TEXTURE_BINDING,
    );
    let denoised = gpu::texture(
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
        usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
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
    let denoise_pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
        label: None,
        layout: None,
        module: &shader,
        entry_point: Some("denoise"),
        compilation_options: Default::default(),
        cache: None,
    });
    let denoised_view = denoised.create_view(&Default::default());
    let denoise_bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: None,
        layout: &denoise_pipeline.get_bind_group_layout(0),
        entries: &[
            gpu::view(0, &depth_view),
            gpu::view(3, &output_view),
            gpu::view(4, &denoised_view),
        ],
    });
    // Device depth is affine across a projected plane. A sloped fixture catches
    // view-dependent bending that a front-facing plane alone would miss.
    let depth_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("Sloped plane depth fixture"),
        source: wgpu::ShaderSource::Wgsl(
            r#"
            @group(0) @binding(0) var<uniform> plane: vec4<f32>;
            @vertex fn vs(@builtin(vertex_index) i: u32) -> @builtin(position) vec4<f32> {
                let p = array<vec2<f32>,3>(vec2(-1.,-1.),vec2(3.,-1.),vec2(-1.,3.));
                return vec4(p[i],0.,1.);
            }
            @fragment fn fs(@builtin(position) p: vec4<f32>) -> @builtin(frag_depth) f32 {
                if plane.w > 0. && p.x < 12. && p.y > 16. { return plane.x - .03; }
                return plane.x + dot(p.xy / 32. - .5, plane.yz);
            }
        "#
            .into(),
        ),
    });
    let depth_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: None,
        layout: None,
        vertex: wgpu::VertexState {
            module: &depth_shader,
            entry_point: Some("vs"),
            compilation_options: Default::default(),
            buffers: &[],
        },
        fragment: Some(wgpu::FragmentState {
            module: &depth_shader,
            entry_point: Some("fs"),
            compilation_options: Default::default(),
            targets: &[],
        }),
        depth_stencil: Some(wgpu::DepthStencilState {
            format: wgpu::TextureFormat::Depth32Float,
            depth_write_enabled: Some(true),
            depth_compare: Some(wgpu::CompareFunction::Always),
            stencil: Default::default(),
            bias: Default::default(),
        }),
        primitive: Default::default(),
        multisample: Default::default(),
        multiview_mask: None,
        cache: None,
    });
    let plane = device.create_buffer(&wgpu::BufferDescriptor {
        label: None,
        size: 16,
        usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    let plane_bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: None,
        layout: &depth_pipeline.get_bind_group_layout(0),
        entries: &[wgpu::BindGroupEntry {
            binding: 0,
            resource: plane.as_entire_binding(),
        }],
    });
    for (distance, slope, intensity, frame, occluder) in [
        (0.18, 0.0, 1.0, 0, false),
        (0.4, 0.0, 1.0, 0, false),
        (2.0, 0.0, 1.0, 0, false),
        (2.0, 0.07, 1.0, 3, false),
        (2.0, -0.07, 1.0, 19, false),
        (2.0, 0.07, 0.0, 7, false),
        (2.0, 0.0, 1.0, 7, true),
        (2.0, 0.0, 0.0, 7, true),
    ] {
        let clip = proj * glam::Vec4::new(0.0, 0.0, -distance, 1.0);
        queue.write_buffer(
            &plane,
            0,
            bytemuck::cast_slice(&[clip.z / clip.w, slope, slope * 0.2, f32::from(occluder)]),
        );
        queue.write_buffer(
            &uniform,
            0,
            bytemuck::bytes_of(&Params {
                projection: proj.to_cols_array_2d(),
                inverse: proj.inverse().to_cols_array_2d(),
                resolution: [1.0 / size as f32; 2],
                radius: 1.0,
                power: 2.0,
                intensity,
                frame,
                near: 0.1,
                grain: 0,
            }),
        );
        let mut encoder = device.create_command_encoder(&Default::default());
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
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
            pass.set_pipeline(&depth_pipeline);
            pass.set_bind_group(0, &plane_bind, &[]);
            pass.draw(0..3, 0..1);
        }
        {
            let mut pass = encoder.begin_compute_pass(&Default::default());
            pass.set_pipeline(&pipeline);
            pass.set_bind_group(0, &bind, &[]);
            pass.dispatch_workgroups(size.div_ceil(8), size.div_ceil(8), 1);
            pass.set_pipeline(&denoise_pipeline);
            pass.set_bind_group(0, &denoise_bind, &[]);
            pass.dispatch_workgroups(size.div_ceil(8), size.div_ceil(8), 1);
        }
        let bytes = gpu::read_rgba(&device, &queue, encoder, &denoised);
        let minimum = bytes.chunks_exact(4).map(|pixel| pixel[3]).min().unwrap();
        println!(
            "plane {distance}m, slope {slope}, intensity {intensity}, occluder {occluder}: visibility {minimum}/255"
        );
        if occluder && intensity > 0.0 {
            assert!(
                minimum < 245,
                "A nearby occluder stopped producing contact AO"
            );
            assert!(
                bytes
                    .chunks_exact(4)
                    .any(|p| p[..3].iter().any(|&v| v.abs_diff(128) > 10)),
                "A nearby occluder stopped bending ambient light"
            );
            // Occluder is at lower-left in image space; open directions point
            // right and up in view space, not toward the occluder.
            let bend: [i32; 2] = std::array::from_fn(|axis| {
                bytes
                    .chunks_exact(4)
                    .enumerate()
                    .filter(|(i, p)| (i % 32 >= 14 || i / 32 <= 14) && p[2] > 180)
                    .map(|(_, p)| i32::from(p[axis]) * 2 - 255)
                    .sum()
            });
            assert!(
                bend[0] > 0 && bend[1] > 0,
                "Bent normal faces the occluder: {bend:?}"
            );
            continue;
        }
        assert!(
            minimum >= 254,
            "An open surface became dark at {distance}m: {minimum}"
        );
        assert!(
            bytes
                .chunks_exact(4)
                .all(|p| p[..3].iter().all(|&v| v.abs_diff(128) <= 1)),
            "An open plane acquired a bent-normal override at {distance}m"
        );
    }
}
