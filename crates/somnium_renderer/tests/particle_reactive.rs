//! Actual particle MRT coverage must respect sprite alpha, depth and empty frames.
use somnium_renderer::{
    pass::particle::{GpuParticle, ParticlePass},
    shaders::Shaders,
};
#[path = "support/gpu.rs"]
mod gpu;

#[test]
#[ignore = "Requires a bindless-capable graphics adapter; run for particle renderer changes"]
fn visible_sprite_coverage_respects_alpha_depth_empty_frames_and_viewports() {
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
    let adapter = gpu::block_on(instance.request_adapter(&Default::default())).unwrap();
    let (device, queue) = gpu::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
        required_features: wgpu::Features::TEXTURE_BINDING_ARRAY
            | wgpu::Features::SAMPLED_TEXTURE_AND_STORAGE_BUFFER_ARRAY_NON_UNIFORM_INDEXING,
        required_limits: adapter.limits(),
        ..Default::default()
    }))
    .unwrap();
    let size = 16;
    let sprite = gpu::texture(
        &device,
        4,
        wgpu::TextureFormat::Rgba8Unorm,
        wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
    );
    let texels: Vec<u8> = (0..16)
        .flat_map(|i| [255, 255, 255, if i % 4 < 2 { 0 } else { 255 }])
        .collect();
    queue.write_texture(
        sprite.as_image_copy(),
        &texels,
        wgpu::TexelCopyBufferLayout {
            offset: 0,
            bytes_per_row: Some(16),
            rows_per_image: Some(4),
        },
        sprite.size(),
    );
    let sprite_view = sprite.create_view(&Default::default());
    let global_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: None,
        entries: &[wgpu::BindGroupLayoutEntry {
            binding: 4,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Texture {
                sample_type: wgpu::TextureSampleType::Float { filterable: true },
                view_dimension: wgpu::TextureViewDimension::D2,
                multisampled: false,
            },
            count: std::num::NonZeroU32::new(1),
        }],
    });
    let global = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: None,
        layout: &global_layout,
        entries: &[wgpu::BindGroupEntry {
            binding: 4,
            resource: wgpu::BindingResource::TextureViewArray(&[&sprite_view]),
        }],
    });
    let hdr = gpu::texture(
        &device,
        size,
        wgpu::TextureFormat::Rgba16Float,
        wgpu::TextureUsages::RENDER_ATTACHMENT,
    );
    let depth = gpu::texture(
        &device,
        size,
        wgpu::TextureFormat::Depth32Float,
        wgpu::TextureUsages::RENDER_ATTACHMENT,
    );
    let hdr_view = hdr.create_view(&Default::default());
    let depth_view = depth.create_view(&Default::default());
    let mut pass = ParticlePass::new(&device, &Shaders::new(), &global_layout);
    let particle = GpuParticle {
        position: [0.0, 0.0, 0.5],
        size: 1.0,
        color: [1.0; 4],
        tip_tint: [1.0; 3],
        aspect: 1.0,
        uv_rect: [0.0, 0.0, 1.0, 1.0],
        rotation: 0.0,
        texture_index: 0,
        flags: 0,
        flutter: 0.0,
    };
    let mut frame = |slot, depth_value, particles: &[GpuParticle]| {
        let mut encoder = device.create_command_encoder(&Default::default());
        {
            let _clear = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: None,
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &depth_view,
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Clear(depth_value),
                        store: wgpu::StoreOp::Store,
                    }),
                    stencil_ops: None,
                }),
                ..Default::default()
            });
        }
        pass.record(
            &device,
            &queue,
            &mut encoder,
            &hdr_view,
            &depth_view,
            &global,
            glam::Mat4::IDENTITY,
            glam::Mat4::IDENTITY,
            slot,
            [size; 2],
            particles,
        );
        gpu::read_rgba(
            &device,
            &queue,
            encoder,
            pass.reactive_view(slot).unwrap().texture(),
        )
    };
    let visible = frame(0, 0.75, &[particle]);
    let pixel = |image: &[u8], x, y| image[((y * size + x) * 4) as usize];
    assert_eq!(
        pixel(&visible, 10, 8),
        255,
        "opaque half of textured sprite is reactive"
    );
    assert_eq!(
        pixel(&visible, 5, 8),
        0,
        "transparent corners keep static history"
    );
    assert_eq!(pixel(&visible, 1, 1), 0, "outside sprite is not reactive");
    let mirrored = frame(
        0,
        0.75,
        &[GpuParticle {
            flags: 4,
            ..particle
        }],
    );
    for y in 4..12 {
        for x in 4..12 {
            assert_eq!(
                pixel(&visible, x, y),
                pixel(&mirrored, 15 - x, y),
                "a staged reflection must reverse the actual asymmetric sprite"
            );
        }
    }
    let bent = frame(
        0,
        0.75,
        &[GpuParticle {
            flutter: 0.4,
            ..particle
        }],
    );
    assert_ne!(
        visible, bent,
        "flutter must deform the actual textured silhouette"
    );
    for x in 4..12 {
        assert_eq!(
            pixel(&visible, x, 11),
            pixel(&bent, x, 11),
            "base stays pinned"
        );
    }
    let occluded = frame(1, 0.25, &[particle]);
    assert!(
        occluded.chunks_exact(4).all(|p| p[0] == 0),
        "depth-hidden sprites must not erase scene history"
    );
    let stopped = frame(0, 0.75, &[]);
    assert!(
        stopped.chunks_exact(4).all(|p| p[0] == 0),
        "empty frame clears previous sprite attachment"
    );
    let additive = frame(
        0,
        0.75,
        &[GpuParticle {
            flags: 1,
            ..particle
        }],
    );
    assert_eq!(
        pixel(&additive, 10, 8),
        255,
        "additive colour alpha zero must still mark visible flame"
    );
}
