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

#[test]
fn a_body_arriving_or_leaving_skins_only_the_newcomer() {
    let (device, queue) = gpu::device_with(
        wgpu::Features::TEXTURE_BINDING_ARRAY | wgpu::Features::SAMPLED_TEXTURE_AND_STORAGE_BUFFER_ARRAY_NON_UNIFORM_INDEXING,
    );
    let shaders = crate::shaders::Shaders::new();
    let mut pool = GeometryPool::new(&device);
    let mut pass = SkinPass::new(&device, &shaders);
    let mut animated = AnimatedGeometry::default();
    let (skeleton, skin) = one_joint();
    let register = |animated: &mut AnimatedGeometry, pool: &mut GeometryPool, pass: &mut SkinPass, x: f32| {
        let rest = pool.upload_mesh(&queue, &triangle(x), &[0, 1, 2], 0);
        animated
            .register_with(&device, &queue, pool, pass, &skeleton, &skin, rest.vertex_offset, &triangle(x))
            .expect("registered")
    };
    let bodies: Vec<_> = [0.0, 10.0, 20.0].into_iter().map(|x| register(&mut animated, &mut pool, &mut pass, x)).collect();
    let frame = |animated: &mut AnimatedGeometry, pool: &mut GeometryPool, pass: &mut SkinPass| {
        animated.prepare(pool);
        let mut encoder = device.create_command_encoder(&Default::default());
        animated.record_with(&device, &queue, &mut encoder, pool, pass);
        queue.submit([encoder.finish()]);
        animated.skinned_last_frame()
    };
    assert_eq!(frame(&mut animated, &mut pool, &mut pass), 3);
    assert_eq!(frame(&mut animated, &mut pool, &mut pass), 0);
    assert!(animated.remove(bodies[1].0, &mut pool));
    let late = register(&mut animated, &mut pool, &mut pass, 30.0);
    assert_eq!(frame(&mut animated, &mut pool, &mut pass), 1, "only the newcomer; the others keep their poses");
    let posed = |pool: &GeometryPool, offset: u32| xs(&device, &queue, &pool.vertex_buffer, offset, 3);
    assert_eq!(posed(&pool, bodies[0].1), [0.0, 1.0, 0.0]);
    assert_eq!(posed(&pool, bodies[2].1), [20.0, 21.0, 20.0]);
    assert_eq!(posed(&pool, late.1), [30.0, 31.0, 30.0]);
    let base = |animated: &AnimatedGeometry, offset: u32| animated.previous_bases().find(|(o, _)| *o == offset).unwrap().1;
    let previous = |animated: &AnimatedGeometry, offset: u32| {
        xs(&device, &queue, animated.previous_pose().unwrap().0, base(animated, offset), 3)
    };
    assert_eq!(previous(&animated, bodies[2].1), [20.0, 21.0, 20.0], "an old body's last pose is where it was");
    assert_eq!(previous(&animated, late.1), [30.0, 31.0, 30.0], "the newcomer starts still");
}

#[test]
fn a_skin_that_does_not_fit_is_refused_before_anything_is_allocated() {
    let (device, queue) = gpu::device_with(
        wgpu::Features::TEXTURE_BINDING_ARRAY | wgpu::Features::SAMPLED_TEXTURE_AND_STORAGE_BUFFER_ARRAY_NON_UNIFORM_INDEXING,
    );
    let shaders = crate::shaders::Shaders::new();
    let mut pool = GeometryPool::new(&device);
    let mut pass = SkinPass::new(&device, &shaders);
    let mut animated = AnimatedGeometry::default();
    let (skeleton, mut skin) = one_joint();
    skin.bindings[1] = SkinBinding::from_influences(&[(3, 1.0)]);
    let rest = pool.upload_mesh(&queue, &triangle(0.0), &[0, 1, 2], 0);
    let error = animated
        .register_with(&device, &queue, &mut pool, &mut pass, &skeleton, &skin, rest.vertex_offset, &triangle(0.0))
        .unwrap_err();
    assert!(error.contains("SkinDoesNotFitSkeleton"), "{error}");
    let (_, short) = one_joint();
    let error = animated
        .register_with(&device, &queue, &mut pool, &mut pass, &skeleton, &short, rest.vertex_offset, &triangle(0.0)[..2])
        .unwrap_err();
    assert!(error.contains("BindingCountMismatch"), "{error}");
}

/// What registering costs as a crowd grows: the first and the last few hundred of 3,000 bodies of 68 joints.
/// Ignored; `cargo test --release -p somnium_renderer --lib animated_geometry::gpu_tests::registering -- --ignored
/// --nocapture`.
#[test]
#[ignore = "a measurement"]
fn registering_a_crowd() {
    let (device, queue) = gpu::device_with(
        wgpu::Features::TEXTURE_BINDING_ARRAY | wgpu::Features::SAMPLED_TEXTURE_AND_STORAGE_BUFFER_ARRAY_NON_UNIFORM_INDEXING,
    );
    let shaders = crate::shaders::Shaders::new();
    let mut pool = GeometryPool::new(&device);
    let mut pass = SkinPass::new(&device, &shaders);
    let mut animated = AnimatedGeometry::default();
    let joints = 68;
    let skeleton = Skeleton::new(
        SkeletonId(0),
        (0..joints).map(|i| format!("j{i}")).collect(),
        (0..joints).map(|i| if i == 0 { NO_PARENT } else { (i - 1) as u16 }).collect(),
        vec![Mat4::IDENTITY; joints],
        vec![Transform::IDENTITY; joints],
    )
    .expect("a chain")
    .0;
    let vertices: Vec<somnium_asset::Vertex> = (0..200).flat_map(|k| triangle(k as f32)).collect();
    let skin = Skin {
        skeleton: SkeletonId(0),
        bindings: (0..vertices.len()).map(|v| SkinBinding::from_influences(&[((v % joints) as u16, 1.0)])).collect(),
    };
    let indices: Vec<u32> = (0..vertices.len() as u32).collect();
    let mut times = Vec::new();
    for _ in 0..3000 {
        let rest = pool.upload_mesh(&queue, &vertices, &indices, 0);
        let started = std::time::Instant::now();
        animated
            .register_with(&device, &queue, &mut pool, &mut pass, &skeleton, &skin, rest.vertex_offset, &vertices)
            .expect("registered");
        times.push(started.elapsed().as_secs_f64() * 1000.0);
    }
    let mean = |s: &[f64]| s.iter().sum::<f64>() / s.len() as f64;
    let (first, last) = (mean(&times[..300]), mean(&times[2700..]));
    println!("register: first 300 {first:.3} ms each, last 300 {last:.3} ms each ({} bodies)", times.len());
    assert!(last < first * 4.0 + 0.5, "registration grows with the crowd");
}
