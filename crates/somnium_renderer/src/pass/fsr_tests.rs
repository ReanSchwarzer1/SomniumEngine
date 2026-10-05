//! The actual sanitize pipeline feeds current and previous dynamic coverage to FSR.
use super::*;
use wgpu::util::DeviceExt;
#[path = "../../tests/support/gpu.rs"]
mod gpu;

#[test]
#[ignore = "Requires a graphics adapter; run for FSR integration changes"]
fn dynamic_mask_covers_motion_removal_camera_reprojection_and_reset() {
    let (device, queue) = gpu::device();
    let size = 16;
    let sampled = wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST;
    let src = gpu::texture(&device, size, wgpu::TextureFormat::Rgba16Float, sampled);
    let dst = alloc_hdr(&device, "Fixture sanitized", [size; 2]);
    let depth = gpu::texture(
        &device,
        size,
        wgpu::TextureFormat::Depth32Float,
        wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::RENDER_ATTACHMENT,
    );
    let depth_float = alloc_r32(&device, "Fixture depth", [size; 2]);
    let visibility = gpu::texture(&device, size, wgpu::TextureFormat::Rg32Uint, sampled);
    let motion = gpu::texture(&device, size, wgpu::TextureFormat::Rg32Float, sampled);
    let sprites = gpu::texture(&device, size, wgpu::TextureFormat::Rgba8Unorm, sampled);
    let coverage = alloc_coverage(&device, [size; 2]);
    let mask = gpu::texture(
        &device,
        size,
        wgpu::TextureFormat::R32Float,
        wgpu::TextureUsages::STORAGE_BINDING | wgpu::TextureUsages::COPY_SRC,
    );
    let flags = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: None,
        contents: bytemuck::cast_slice(&[0_u32, 1]),
        usage: wgpu::BufferUsages::STORAGE,
    });
    let uniform = device.create_buffer(&wgpu::BufferDescriptor {
        label: None,
        size: 16,
        usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    let (layout, pipeline) = alloc_sanitize_pipeline(&device, &crate::shaders::Shaders::new());
    let sample = |bytes: &[u8], x: u32, y: u32| {
        let offset = ((y * size + x) * 4) as usize;
        f32::from_le_bytes(bytes[offset..offset + 4].try_into().unwrap())
    };
    let frame = |index: usize,
                 actor: Option<(u32, u32)>,
                 motion_x: f32,
                 valid: bool,
                 sprite: Option<(u32, u32)>| {
        let ids: Vec<u32> = (0..size)
            .flat_map(|y| {
                (0..size).flat_map(move |x| [if actor == Some((x, y)) { 2 } else { 1 }, 0])
            })
            .collect();
        let sprite_pixels: Vec<u8> = (0..size)
            .flat_map(|y| {
                (0..size).flat_map(move |x| [if sprite == Some((x, y)) { 255 } else { 0 }, 0, 0, 0])
            })
            .collect();
        queue.write_texture(
            sprites.as_image_copy(),
            &sprite_pixels,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(size * 4),
                rows_per_image: Some(size),
            },
            sprites.size(),
        );
        let motions: Vec<f32> = (0..size * size).flat_map(|_| [motion_x, 0.0]).collect();
        for (texture, bytes) in [
            (&visibility, bytemuck::cast_slice(ids.as_slice())),
            (&motion, bytemuck::cast_slice(motions.as_slice())),
        ] {
            queue.write_texture(
                texture.as_image_copy(),
                bytes,
                wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(size * 8),
                    rows_per_image: Some(size),
                },
                texture.size(),
            );
        }
        queue.write_buffer(
            &uniform,
            0,
            bytemuck::bytes_of(&[1.0_f32, 0.0, if valid { 1.0 } else { 0.0 }, 0.0]),
        );
        let views: Vec<_> = [
            &src,
            &dst,
            &depth,
            &depth_float,
            &visibility,
            &coverage[1 - index % 2],
            &coverage[index % 2],
            &mask,
            &motion,
            &sprites,
        ]
        .map(|texture| texture.create_view(&Default::default()))
        .into();
        let bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: None,
            layout: &layout,
            entries: &[
                bind_view(0, &views[0]),
                bind_view(1, &views[1]),
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: uniform.as_entire_binding(),
                },
                bind_view(3, &views[2]),
                bind_view(4, &views[3]),
                bind_view(5, &views[4]),
                wgpu::BindGroupEntry {
                    binding: 6,
                    resource: flags.as_entire_binding(),
                },
                bind_view(7, &views[5]),
                bind_view(8, &views[6]),
                bind_view(9, &views[7]),
                bind_view(10, &views[8]),
                bind_view(11, &views[9]),
            ],
        });
        let mut encoder = device.create_command_encoder(&Default::default());
        {
            let _pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: None,
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &views[2],
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Clear(if actor.is_some() { 0.2 } else { 0.95 }),
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
        gpu::read_rgba(&device, &queue, encoder, &mask)
    };
    let first = frame(0, Some((4, 4)), 0.0, false, None);
    assert_eq!(sample(&first, 4, 4), 1.0);
    assert_eq!(sample(&first, 3, 3), 1.0, "jitter footprint");
    assert_eq!(sample(&first, 14, 2), 0.0, "static scene must keep history");
    let moved = frame(1, Some((11, 11)), 0.0, true, None);
    assert_eq!(
        sample(&moved, 4, 4),
        1.0,
        "old silhouette must reject history"
    );
    assert_eq!(
        sample(&moved, 11, 11),
        1.0,
        "new silhouette must reject history"
    );
    let removed = frame(2, None, 0.0, true, None);
    assert_eq!(
        sample(&removed, 11, 11),
        1.0,
        "removed actor, new background depth"
    );
    assert_eq!(
        sample(&removed, 4, 4),
        0.0,
        "coverage must not accumulate its own history"
    );
    let settled = frame(3, None, 0.0, true, None);
    assert!(
        settled
            .chunks_exact(4)
            .all(|bytes| f32::from_le_bytes(bytes.try_into().unwrap()) == 0.0)
    );
    frame(4, Some((4, 4)), 0.0, true, None);
    let camera = frame(5, None, -0.25, true, None);
    assert_eq!(
        sample(&camera, 8, 4),
        1.0,
        "camera reprojection carries old coverage"
    );
    frame(6, Some((4, 4)), 0.0, true, None);
    let reset = frame(7, None, 0.0, false, None);
    assert_eq!(
        sample(&reset, 4, 4),
        0.0,
        "resize/enable reset ignores prior coverage"
    );
    let smoke = frame(8, None, 0.0, true, Some((13, 3)));
    assert_eq!(
        sample(&smoke, 13, 3),
        1.0,
        "standalone smoke has no opaque instance ID"
    );
    assert_eq!(
        sample(&smoke, 2, 13),
        0.0,
        "smoke does not erase unrelated static history"
    );
    let vanished = frame(9, None, 0.0, true, None);
    assert_eq!(
        sample(&vanished, 13, 3),
        1.0,
        "vanished smoke coverage rejects old FSR history"
    );
    let clear = frame(10, None, 0.0, true, None);
    assert_eq!(
        sample(&clear, 13, 3),
        0.0,
        "vanished smoke coverage expires"
    );
}

#[test]
#[ignore = "Requires an FSR-capable adapter; validates the actual backend dispatch"]
fn fsr_backend_accepts_dynamic_masks_and_resize() {
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
    let adapter = gpu::block_on(instance.request_adapter(&Default::default())).unwrap();
    let (device, queue) = gpu::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
        required_features: crate::context::FSR_FEATURES,
        required_limits: adapter.limits(),
        ..Default::default()
    }))
    .unwrap();
    let mut fsr = FsrPass::new(
        &device,
        &crate::shaders::Shaders::new(),
        &queue,
        64,
        64,
        128,
        128,
    );
    fsr.set_enabled(true);
    for size in [256, 128] {
        fsr.resize(&device, &queue, size, size, size * 2, size * 2);
        // Exercise growth and shrinking the active set between dispatches.
        fsr.set_reactive_instances(&device, &queue, &[0, 1, 0, 1]);
        fsr.set_reactive_instances(&device, &queue, &[0, 1]);
        let color = alloc_hdr(&device, "Backend input", [size; 2]);
        let motion = alloc_rg16(&device, "Backend motion", [size; 2]);
        let visibility = gpu::texture(
            &device,
            size,
            wgpu::TextureFormat::Rg32Uint,
            wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        );
        let ids: Vec<u32> = (0..size * size).flat_map(|_| [2, 0]).collect();
        queue.write_texture(
            visibility.as_image_copy(),
            bytemuck::cast_slice(&ids),
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(size * 8),
                rows_per_image: Some(size),
            },
            visibility.size(),
        );
        let depth = gpu::texture(
            &device,
            size,
            wgpu::TextureFormat::Depth32Float,
            wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::RENDER_ATTACHMENT,
        );
        let depth_view = depth.create_view(&Default::default());
        for _ in 0..2 {
            let mut encoder = device.create_command_encoder(&Default::default());
            {
                let _pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: None,
                    depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                        view: &depth_view,
                        depth_ops: Some(wgpu::Operations {
                            load: wgpu::LoadOp::Clear(0.5),
                            store: wgpu::StoreOp::Store,
                        }),
                        stencil_ops: None,
                    }),
                    ..Default::default()
                });
            }
            assert!(fsr.record(
                &device,
                &queue,
                &mut encoder,
                &color,
                &depth,
                &motion,
                &visibility.create_view(&Default::default()),
                None,
                1.0,
                glam::Mat4::perspective_rh(1.0, 1.0, 0.1, 1000.0),
                1.0 / 60.0,
                false
            ));
            queue.submit([encoder.finish()]);
            device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
        }
    }
}
