//! Actual production mask/composite coverage, including transparent depth and view isolation.
use bytemuck::Zeroable;
use somnium_renderer::{
    instance::GpuInstanceData,
    material::pool::GpuMaterial,
    pass::{
        game_highlight::{GameHighlightPass, HighlightDraw, HighlightStyle},
        postprocess::Grading,
    },
    shaders::Shaders,
};
use wgpu::util::DeviceExt;
#[allow(dead_code)]
#[path = "support/gpu.rs"]
mod gpu;

#[test]
#[ignore = "Requires a bindless graphics adapter; coordinate with native review"]
fn silhouettes_union_parts_respect_occlusion_alpha_and_view_lifetimes() {
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
    let adapter = gpu::block_on(instance.request_adapter(&Default::default())).unwrap();
    let (device, queue) = gpu::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
        required_features: wgpu::Features::TEXTURE_BINDING_ARRAY
            | wgpu::Features::SAMPLED_TEXTURE_AND_STORAGE_BUFFER_ARRAY_NON_UNIFORM_INDEXING,
        required_limits: adapter.limits(),
        ..Default::default()
    }))
    .unwrap();
    let size = 32;
    let vertices: [[f32; 8]; 4] = [
        [-0.5, -0.5, 0.6, 0.0, 0.0, -1.0, 0.0, 1.0],
        [0.5, -0.5, 0.6, 0.0, 0.0, -1.0, 1.0, 1.0],
        [0.5, 0.5, 0.6, 0.0, 0.0, -1.0, 1.0, 0.0],
        [-0.5, 0.5, 0.6, 0.0, 0.0, -1.0, 0.0, 0.0],
    ];
    let buffer = |bytes: &[u8]| {
        device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: None,
            contents: bytes,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
        })
    };
    let vertex_buffer = buffer(bytemuck::cast_slice(&vertices));
    let index_buffer = buffer(bytemuck::cast_slice(&[0u32, 1, 2, 0, 2, 3]));
    let instances = buffer(bytemuck::bytes_of(&GpuInstanceData {
        model_matrix: glam::Mat4::IDENTITY.to_cols_array_2d(),
        material_id: 0,
        mesh_vertex_offset: 0,
        mesh_index_offset: 0,
        _padding: 0,
    }));
    let mut view_bytes = vec![0u8; 256];
    view_bytes[..64].copy_from_slice(bytemuck::cast_slice(&glam::Mat4::IDENTITY.to_cols_array()));
    let view_buffer = buffer(&view_bytes);
    let mut material = GpuMaterial::zeroed();
    material.base_color = [1.0; 4];
    material.albedo_map = 0;
    let material_buffer = buffer(bytemuck::bytes_of(&material));
    let alpha = gpu::texture(
        &device,
        4,
        wgpu::TextureFormat::Rgba8Unorm,
        wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
    );
    let texels: Vec<u8> = (0..16)
        .flat_map(|i| [255, 255, 255, if i % 4 < 2 { 0 } else { 255 }])
        .collect();
    queue.write_texture(
        alpha.as_image_copy(),
        &texels,
        wgpu::TexelCopyBufferLayout {
            offset: 0,
            bytes_per_row: Some(16),
            rows_per_image: Some(4),
        },
        alpha.size(),
    );
    let alpha_view = alpha.create_view(&Default::default());
    let mut entries: Vec<_> = [0, 1, 2, 3, 5]
        .into_iter()
        .map(|binding| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
            ty: wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Storage { read_only: true },
                has_dynamic_offset: false,
                min_binding_size: None,
            },
            count: None,
        })
        .collect();
    entries.push(wgpu::BindGroupLayoutEntry {
        binding: 4,
        visibility: wgpu::ShaderStages::FRAGMENT,
        ty: wgpu::BindingType::Texture {
            sample_type: wgpu::TextureSampleType::Float { filterable: true },
            view_dimension: wgpu::TextureViewDimension::D2,
            multisampled: false,
        },
        count: std::num::NonZeroU32::new(1),
    });
    let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: None,
        entries: &entries,
    });
    let global = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: None,
        layout: &layout,
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: vertex_buffer.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: index_buffer.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 2,
                resource: instances.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 3,
                resource: view_buffer.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 4,
                resource: wgpu::BindingResource::TextureViewArray(&[&alpha_view]),
            },
            wgpu::BindGroupEntry {
                binding: 5,
                resource: material_buffer.as_entire_binding(),
            },
        ],
    });
    let vis = gpu::texture(
        &device,
        size,
        wgpu::TextureFormat::Rg32Uint,
        wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
    );
    let vis_view = vis.create_view(&Default::default());
    let depth = gpu::texture(
        &device,
        size,
        wgpu::TextureFormat::Depth32Float,
        wgpu::TextureUsages::RENDER_ATTACHMENT,
    );
    let depth_view = depth.create_view(&Default::default());
    let output = gpu::texture(
        &device,
        size,
        wgpu::TextureFormat::Rgba8Unorm,
        wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
    );
    let output_view = output.create_view(&Default::default());
    let mut pass = GameHighlightPass::new(
        &device,
        &Shaders::new(),
        &layout,
        wgpu::TextureFormat::Rgba8Unorm,
        size,
        size,
    );
    let upload_vis = |filled: bool| {
        let values: Vec<u32> = (0..size)
            .flat_map(|y| {
                (0..size).flat_map(move |x| {
                    let id = if filled && (4..28).contains(&x) && (4..28).contains(&y) {
                        if (13..19).contains(&x) && (13..19).contains(&y) {
                            3
                        } else if x < 16 {
                            1
                        } else {
                            2
                        }
                    } else {
                        0
                    };
                    [id, 0]
                })
            })
            .collect();
        queue.write_texture(
            vis.as_image_copy(),
            bytemuck::cast_slice(&values),
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(size * 8),
                rows_per_image: Some(size),
            },
            vis.size(),
        );
    };
    let clear = |encoder: &mut wgpu::CommandEncoder, target: &wgpu::TextureView, distance| {
        let _pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: None,
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: target,
                resolve_target: None,
                depth_slice: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                view: &depth_view,
                depth_ops: Some(wgpu::Operations {
                    load: wgpu::LoadOp::Clear(distance),
                    store: wgpu::StoreOp::Store,
                }),
                stencil_ops: None,
            }),
            ..Default::default()
        });
    };
    let pixel = |image: &[u8], x: u32, y: u32| image[((y * size + x) * 4) as usize];
    upload_vis(true);
    let mut encoder = device.create_command_encoder(&Default::default());
    clear(&mut encoder, &output_view, 1.0);
    pass.record_mask(
        &device,
        &queue,
        &mut encoder,
        &global,
        &vis_view,
        &depth_view,
        &[1, 1, 0],
        &[],
        0,
    );
    pass.composite(
        &device,
        &queue,
        &mut encoder,
        &output_view,
        [size; 2],
        None,
        HighlightStyle {
            color: [1.0; 4],
            width_pixels: 2.0,
            show_mask: false,
        },
        Grading::default(),
        [size; 2],
        [0.0; 2],
        0,
    );
    let image = gpu::read_rgba(&device, &queue, encoder, &output);
    assert!(pixel(&image, 4, 8) > 200, "visible silhouette edge");
    assert_eq!(
        pixel(&image, 16, 8),
        0,
        "adjacent selected parts must not have a seam"
    );
    assert_eq!(
        pixel(&image, 15, 15),
        0,
        "foreground occluder never receives an outer halo"
    );
    assert_eq!(
        pixel(&image, 3, 8),
        0,
        "border stays inside visible coverage"
    );
    upload_vis(false);
    for (distance, expected) in [(0.8, true), (0.4, false)] {
        let mut encoder = device.create_command_encoder(&Default::default());
        clear(&mut encoder, &output_view, distance);
        pass.record_mask(
            &device,
            &queue,
            &mut encoder,
            &global,
            &vis_view,
            &depth_view,
            &[],
            &[HighlightDraw {
                instance_index: 0,
                index_count: 6,
            }],
            0,
        );
        pass.composite(
            &device,
            &queue,
            &mut encoder,
            &output_view,
            [size; 2],
            None,
            HighlightStyle {
                color: [1.0; 4],
                show_mask: true,
                ..Default::default()
            },
            Grading::default(),
            [size; 2],
            [0.0; 2],
            0,
        );
        let image = gpu::read_rgba(&device, &queue, encoder, &output);
        assert_eq!(
            pixel(&image, 21, 16) > 200,
            expected,
            "transparent planar geometry must depth test"
        );
        assert_eq!(
            pixel(&image, 10, 16),
            0,
            "transparent texture hole must remain clear"
        );
    }
    upload_vis(true);
    let second = gpu::texture(
        &device,
        size,
        wgpu::TextureFormat::Rgba8Unorm,
        wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
    );
    let second_view = second.create_view(&Default::default());
    let mut encoder = device.create_command_encoder(&Default::default());
    clear(&mut encoder, &output_view, 1.0);
    pass.record_mask(
        &device,
        &queue,
        &mut encoder,
        &global,
        &vis_view,
        &depth_view,
        &[1, 0, 0],
        &[],
        0,
    );
    pass.composite(
        &device,
        &queue,
        &mut encoder,
        &output_view,
        [size; 2],
        None,
        HighlightStyle {
            color: [1.0, 0.0, 0.0, 1.0],
            show_mask: true,
            ..Default::default()
        },
        Grading::default(),
        [size; 2],
        [0.0; 2],
        0,
    );
    clear(&mut encoder, &second_view, 1.0);
    pass.record_mask(
        &device,
        &queue,
        &mut encoder,
        &global,
        &vis_view,
        &depth_view,
        &[0, 1, 0],
        &[],
        1,
    );
    pass.composite(
        &device,
        &queue,
        &mut encoder,
        &second_view,
        [size; 2],
        None,
        HighlightStyle {
            color: [0.0, 1.0, 0.0, 1.0],
            show_mask: true,
            ..Default::default()
        },
        Grading::default(),
        [size; 2],
        [0.0; 2],
        1,
    );
    let first = gpu::read_rgba(&device, &queue, encoder, &output);
    assert!(
        pixel(&first, 8, 8) > 200,
        "later view must not replace first-view uniforms/membership"
    );
    assert_eq!(pixel(&first, 24, 8), 0);
    let second = gpu::read_rgba(
        &device,
        &queue,
        device.create_command_encoder(&Default::default()),
        &second,
    );
    assert!(second[((8 * size + 24) * 4 + 1) as usize] > 200);
    assert_eq!(second[((8 * size + 8) * 4 + 1) as usize], 0);

    let mut encoder = device.create_command_encoder(&Default::default());
    clear(&mut encoder, &output_view, 1.0);
    pass.record_mask(
        &device,
        &queue,
        &mut encoder,
        &global,
        &vis_view,
        &depth_view,
        &[0, 0, 0],
        &[],
        0,
    );
    pass.composite(
        &device,
        &queue,
        &mut encoder,
        &output_view,
        [size; 2],
        None,
        HighlightStyle {
            color: [1.0; 4],
            show_mask: true,
            ..Default::default()
        },
        Grading::default(),
        [size; 2],
        [0.0; 2],
        0,
    );
    let cleared = gpu::read_rgba(&device, &queue, encoder, &output);
    assert!(
        cleared.chunks_exact(4).all(|pixel| pixel[..3] == [0; 3]),
        "losing selection clears all previous coverage"
    );

    let small = 16;
    let large = 64;
    pass.resize(small, small);
    let resized_vis = gpu::texture(
        &device,
        small,
        wgpu::TextureFormat::Rg32Uint,
        wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
    );
    let values: Vec<u32> = (0..small)
        .flat_map(|y| {
            (0..small)
                .flat_map(move |x| [u32::from((2..14).contains(&x) && (2..14).contains(&y)), 0])
        })
        .collect();
    queue.write_texture(
        resized_vis.as_image_copy(),
        bytemuck::cast_slice(&values),
        wgpu::TexelCopyBufferLayout {
            offset: 0,
            bytes_per_row: Some(small * 8),
            rows_per_image: Some(small),
        },
        resized_vis.size(),
    );
    let resized_depth = gpu::texture(
        &device,
        small,
        wgpu::TextureFormat::Depth32Float,
        wgpu::TextureUsages::RENDER_ATTACHMENT,
    );
    let large_output = gpu::texture(
        &device,
        large,
        wgpu::TextureFormat::Rgba8Unorm,
        wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
    );
    let large_view = large_output.create_view(&Default::default());
    let mut encoder = device.create_command_encoder(&Default::default());
    {
        let _clear = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: &large_view,
                resolve_target: None,
                depth_slice: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                    store: wgpu::StoreOp::Store,
                },
            })],
            ..Default::default()
        });
    }
    pass.record_mask(
        &device,
        &queue,
        &mut encoder,
        &global,
        &resized_vis.create_view(&Default::default()),
        &resized_depth.create_view(&Default::default()),
        &[1],
        &[],
        0,
    );
    pass.composite(
        &device,
        &queue,
        &mut encoder,
        &large_view,
        [large; 2],
        None,
        HighlightStyle {
            color: [1.0; 4],
            width_pixels: 2.0,
            show_mask: false,
        },
        Grading::default(),
        [large; 2],
        [0.0; 2],
        0,
    );
    let resized = gpu::read_rgba(&device, &queue, encoder, &large_output);
    let red = |x, y| resized[((y * large + x) * 4) as usize];
    assert!(
        red(9, 32) > 20,
        "mask scales to reconstructed display resolution after resize"
    );
    assert_eq!(
        red(14, 32),
        0,
        "display-pixel width must not multiply by the render-scale ratio"
    );
    assert_eq!(
        red(32, 32),
        0,
        "a flat page remains a border, not a filled box"
    );
}
