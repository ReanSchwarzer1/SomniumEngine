//! Skinning only what changed, on a real device: the posed vertices and last frame's pose, read back.
use super::*;
use somnium_anim::{NO_PARENT, SkeletonId, SkinBinding, Transform};

#[allow(dead_code)]
#[path = "../tests/support/gpu.rs"]
mod gpu;

fn one_joint() -> (Skeleton, Skin) {
    let skeleton = Skeleton::new(
        SkeletonId(0),
        vec!["root".into()],
        vec![NO_PARENT],
        vec![Mat4::IDENTITY],
        vec![Transform::IDENTITY],
    )
    .expect("one joint")
    .0;
    let skin = Skin { skeleton: SkeletonId(0), bindings: vec![SkinBinding::from_influences(&[(0, 1.0)]); 3] };
    (skeleton, skin)
}

fn triangle(x: f32) -> Vec<somnium_asset::Vertex> {
    [[x, 0.0, 0.0], [x + 1.0, 0.0, 0.0], [x, 1.0, 0.0]]
        .map(|position| somnium_asset::Vertex { position, normal: [0.0, 0.0, 1.0], uv: [0.0; 2] })
        .to_vec()
}

/// The x of each vertex in `count` vertices from `offset` of `buffer`.
fn xs(device: &wgpu::Device, queue: &wgpu::Queue, buffer: &wgpu::Buffer, offset: u32, count: u32) -> Vec<f32> {
    let bytes = u64::from(count) * VERTEX_BYTES;
    let staging = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("pose readback"),
        size: bytes,
        usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    let mut encoder = device.create_command_encoder(&Default::default());
    encoder.copy_buffer_to_buffer(buffer, u64::from(offset) * VERTEX_BYTES, &staging, 0, bytes);
    queue.submit([encoder.finish()]);
    staging.slice(..).map_async(wgpu::MapMode::Read, |_| {});
    device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
    let data = staging.slice(..).get_mapped_range().unwrap().to_vec();
    let vertices: Vec<somnium_asset::Vertex> = bytemuck::pod_collect_to_vec(&data);
    vertices.iter().map(|v| v.position[0]).collect()
}

#[test]
fn a_still_pose_is_not_skinned_again_and_reads_as_still() {
    // The renderer's own bindless features: `skinning.wgsl` shares the global pool's declarations.
    let (device, queue) = gpu::device_with(
        wgpu::Features::TEXTURE_BINDING_ARRAY
            | wgpu::Features::SAMPLED_TEXTURE_AND_STORAGE_BUFFER_ARRAY_NON_UNIFORM_INDEXING,
    );
    let shaders = crate::shaders::Shaders::new();
    let mut pool = GeometryPool::new(&device);
    let mut pass = SkinPass::new(&device, &shaders);
    let mut animated = AnimatedGeometry::default();
    let (skeleton, skin) = one_joint();
    let mut bodies = Vec::new();
    for x in [0.0, 10.0] {
        let rest = pool.upload_mesh(&queue, &triangle(x), &[0, 1, 2], 0);
        let (id, posed) = animated
            .register_with(&device, &queue, &mut pool, &mut pass, &skeleton, &skin, rest.vertex_offset, &triangle(x))
            .expect("registered");
        bodies.push((id, posed));
    }
    let (walker, stander) = (bodies[0], bodies[1]);
    let mut frame = |animated: &mut AnimatedGeometry, pool: &mut GeometryPool, walker_x: Option<f32>| {
        if let Some(x) = walker_x {
            assert!(animated.update(walker.0, &[Mat4::from_translation(glam::Vec3::new(x, 0.0, 0.0))]));
        }
        // The stander is given the same palette every frame, as a game sampling at a low rate does.
        assert!(animated.update(stander.0, &[Mat4::IDENTITY]));
        animated.prepare(pool);
        let mut encoder = device.create_command_encoder(&Default::default());
        animated.record_with(&device, &queue, &mut encoder, pool, &mut pass);
        queue.submit([encoder.finish()]);
        animated.skinned_last_frame()
    };
    let posed = |pool: &GeometryPool, body: (AnimatedMeshId, u32)| xs(&device, &queue, &pool.vertex_buffer, body.1, 3);
    let previous = |animated: &AnimatedGeometry, base: u32| {
        xs(&device, &queue, animated.previous_pose().expect("a previous pose").0, base, 3)
    };
    let base = |animated: &AnimatedGeometry, body: (AnimatedMeshId, u32)| {
        animated.previous_bases().find(|(offset, _)| *offset == body.1).unwrap().1
    };

    // Frame 1: both new, both skinned; their last pose is this one.
    assert_eq!(frame(&mut animated, &mut pool, None), 2);
    assert_eq!(posed(&pool, walker), [0.0, 1.0, 0.0]);
    assert_eq!(previous(&animated, base(&animated, stander)), [10.0, 11.0, 10.0]);
    // Frame 2: the walker moves 2 m; only it is skinned, and its last pose is where it was.
    assert_eq!(frame(&mut animated, &mut pool, Some(2.0)), 1);
    assert_eq!(posed(&pool, walker), [2.0, 3.0, 2.0]);
    assert_eq!(previous(&animated, base(&animated, walker)), [0.0, 1.0, 0.0]);
    // Frame 3: it holds that pose (a 10 Hz sample): nothing is skinned, and its last pose catches up, so it reads
    // as still.
    assert_eq!(frame(&mut animated, &mut pool, Some(2.0)), 0);
    assert_eq!(posed(&pool, walker), [2.0, 3.0, 2.0]);
    assert_eq!(previous(&animated, base(&animated, walker)), [2.0, 3.0, 2.0]);
    // Frame 4: still nothing, and nothing has drifted.
    assert_eq!(frame(&mut animated, &mut pool, None), 0);
    assert_eq!(posed(&pool, walker), [2.0, 3.0, 2.0]);
    assert_eq!(posed(&pool, stander), [10.0, 11.0, 10.0]);
    assert_eq!(previous(&animated, base(&animated, walker)), [2.0, 3.0, 2.0]);
    // Frame 5: it moves again, from where it stood.
    assert_eq!(frame(&mut animated, &mut pool, Some(5.0)), 1);
    assert_eq!(posed(&pool, walker), [5.0, 6.0, 5.0]);
    assert_eq!(previous(&animated, base(&animated, walker)), [2.0, 3.0, 2.0]);
}
