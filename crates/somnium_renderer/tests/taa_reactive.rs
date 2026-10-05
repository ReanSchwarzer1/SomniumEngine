//! Device-level proof that moving silhouettes do not persist in TAA history.
use somnium_renderer::{pass::taa::TaaPass, shaders::Shaders};
use std::{
    future::Future,
    sync::Arc,
    task::{Context, Poll, Wake, Waker},
};
use wgpu::util::DeviceExt;

fn block_on<T>(future: impl Future<Output = T>) -> T {
    struct ThreadWake(std::thread::Thread);
    impl Wake for ThreadWake {
        fn wake(self: Arc<Self>) {
            self.0.unpark();
        }
    }
    let waker = Waker::from(Arc::new(ThreadWake(std::thread::current())));
    let mut context = Context::from_waker(&waker);
    let mut future = Box::pin(future);
    loop {
        match future.as_mut().poll(&mut context) {
            Poll::Ready(value) => return value,
            Poll::Pending => std::thread::park(),
        }
    }
}
const SIZE: u32 = 16;
fn texture(
    device: &wgpu::Device,
    format: wgpu::TextureFormat,
    usage: wgpu::TextureUsages,
) -> wgpu::Texture {
    device.create_texture(&wgpu::TextureDescriptor {
        label: Some("TAA reactive fixture"),
        size: wgpu::Extent3d {
            width: SIZE,
            height: SIZE,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format,
        usage,
        view_formats: &[],
    })
}
fn pixels(add: u8, actor: Option<(u32, u32)>) -> (Vec<u8>, Vec<u32>) {
    let mut image = vec![];
    let mut ids = vec![];
    for y in 0..SIZE {
        for x in 0..SIZE {
            let value = [32_u8, 100, 210][((x + y) % 3) as usize].saturating_add(add);
            let dynamic = actor == Some((x, y));
            image.extend(if dynamic {
                [220, 10, 30, 255]
            } else {
                [value, value, value, 255]
            });
            ids.extend([if dynamic { 2 } else { 1 }, 0]);
        }
    }
    (image, ids)
}
fn frame(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    taa: &mut TaaPass,
    current: &wgpu::Texture,
    visibility: &wgpu::Texture,
    image: &[u8],
    ids: &[u32],
) -> Vec<u8> {
    for (texture, bytes, stride) in [
        (current, image, SIZE * 4),
        (visibility, bytemuck::cast_slice(ids), SIZE * 8),
    ] {
        queue.write_texture(
            texture.as_image_copy(),
            bytes,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(stride),
                rows_per_image: Some(SIZE),
            },
            texture.size(),
        );
    }
    let read = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("TAA fixture readback"),
        size: u64::from(256 * SIZE),
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let mut encoder = device.create_command_encoder(&Default::default());
    assert!(
        taa.record(&mut encoder, queue, glam::Mat4::IDENTITY, SIZE, SIZE)
            .is_some()
    );
    encoder.copy_texture_to_buffer(
        taa.resolved_texture(taa.last_written()).as_image_copy(),
        wgpu::TexelCopyBufferInfo {
            buffer: &read,
            layout: wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(256),
                rows_per_image: Some(SIZE),
            },
        },
        current.size(),
    );
    queue.submit([encoder.finish()]);
    let (send, recv) = std::sync::mpsc::channel();
    read.slice(..)
        .map_async(wgpu::MapMode::Read, move |result| {
            send.send(result).unwrap();
        });
    device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
    recv.recv().unwrap().unwrap();
    let mapped = read.slice(..).get_mapped_range().unwrap();
    let mut output = vec![];
    for y in 0..SIZE as usize {
        output.extend_from_slice(&mapped[y * 256..y * 256 + SIZE as usize * 4]);
    }
    output
}
fn pixel(image: &[u8], x: u32, y: u32) -> &[u8] {
    let index = ((y * SIZE + x) * 4) as usize;
    &image[index..index + 4]
}

#[test]
#[ignore = "Requires a graphics adapter; run explicitly for temporal renderer changes"]
fn dynamic_coverage_rejects_current_and_previous_history_without_disabling_static_taa() {
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
    let adapter =
        block_on(instance.request_adapter(&wgpu::RequestAdapterOptions::default())).unwrap();
    let (device, queue) =
        block_on(adapter.request_device(&wgpu::DeviceDescriptor::default())).unwrap();
    let sampled = wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST;
    let current = texture(&device, wgpu::TextureFormat::Rgba8Unorm, sampled);
    let visibility = texture(&device, wgpu::TextureFormat::Rg32Uint, sampled);
    let water = texture(&device, wgpu::TextureFormat::Rgba8Unorm, sampled);
    let velocity = texture(&device, wgpu::TextureFormat::Rg32Float, sampled);
    let depth = texture(
        &device,
        wgpu::TextureFormat::Depth32Float,
        wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::RENDER_ATTACHMENT,
    );
    let depth_view = depth.create_view(&Default::default());
    let mut encoder = device.create_command_encoder(&Default::default());
    {
        let _pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("Fixture constant depth"),
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
    queue.submit([encoder.finish()]);
    let exposure = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("Fixture exposure"),
        contents: bytemuck::cast_slice(&[1.0_f32, 1.0]),
        usage: wgpu::BufferUsages::STORAGE,
    });
    let mut taa = TaaPass::new(
        &device,
        &Shaders::new(),
        wgpu::TextureFormat::Rgba8Unorm,
        SIZE,
        SIZE,
        &exposure,
    );
    taa.set_enabled(true);
    taa.set_reactive_instances(&device, &queue, &[0, 1]);
    let bind = |taa: &mut TaaPass| {
        taa.ensure_bind_groups(
            &device,
            &current.create_view(&Default::default()),
            &depth_view,
            &velocity.create_view(&Default::default()),
            &water.create_view(&Default::default()),
            &visibility.create_view(&Default::default()),
        )
    };
    bind(&mut taa);
    let (a, ia) = pixels(0, Some((4, 8)));
    let first = frame(&device, &queue, &mut taa, &current, &visibility, &a, &ia);
    assert_eq!(pixel(&first, 4, 8)[3], 255);
    let (b, ib) = pixels(16, Some((10, 8)));
    let second = frame(&device, &queue, &mut taa, &current, &visibility, &b, &ib);
    for (x, y) in [(4, 8), (10, 8)] {
        assert_eq!(
            &pixel(&second, x, y)[..3],
            &pixel(&b, x, y)[..3],
            "current or old actor must use fresh colour"
        );
    }
    let static_delta = pixel(&second, 8, 5)[0].abs_diff(pixel(&b, 8, 5)[0]);
    assert!(
        static_delta > 3,
        "static pixels must retain history: delta={static_delta}"
    );
    assert_eq!(
        pixel(&second, 4, 8)[3],
        0,
        "previous coverage must not propagate as current coverage"
    );
    let (c, ic) = pixels(16, None);
    let third = frame(&device, &queue, &mut taa, &current, &visibility, &c, &ic);
    assert_eq!(
        &pixel(&third, 10, 8)[..3],
        &pixel(&c, 10, 8)[..3],
        "removing actor clears its former silhouette"
    );
    assert_eq!(pixel(&third, 10, 8)[3], 0);
    taa.resize(&device, wgpu::TextureFormat::Rgba8Unorm, SIZE, SIZE);
    bind(&mut taa);
    let reset = frame(&device, &queue, &mut taa, &current, &visibility, &c, &ic);
    assert_eq!(&pixel(&reset, 8, 5)[..3], &pixel(&c, 8, 5)[..3]);
    println!(
        "current/previous/removed coverage fresh; static history delta={static_delta}; resize clean"
    );
}
