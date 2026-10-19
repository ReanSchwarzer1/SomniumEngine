//! The instance pool past its first 16 MiB, and uploads that send only what changed.
#[allow(dead_code)]
#[path = "support/gpu.rs"]
mod gpu;

use somnium_renderer::instance::{GpuInstanceData, InstancePool};

fn instance(i: u32) -> GpuInstanceData {
    let mut model = glam::Mat4::IDENTITY.to_cols_array_2d();
    model[3][0] = i as f32;
    GpuInstanceData {
        model_matrix: model,
        material_id: i % 7,
        mesh_vertex_offset: i,
        mesh_index_offset: 0,
        _padding: 0,
    }
}

#[test]
fn a_region_of_instances_grows_the_buffer_and_still_frames_upload_nothing() {
    let (device, queue) = gpu::device();
    let mut pool = InstancePool::new(&device);
    let first = pool.buffer.size();
    // 300,000 instances: past the first buffer's 209,715.
    for i in 0..300_000 {
        pool.add_instance(instance(i));
    }
    assert!(
        pool.reserve(&device),
        "the buffer grows instead of being written past its end"
    );
    assert!(pool.buffer.size() >= 300_000 * std::mem::size_of::<GpuInstanceData>() as u64);
    assert!(pool.buffer.size() > first);
    pool.upload(&queue);
    assert_eq!(pool.chunks_written(), 300_000usize.div_ceil(1024));
    // The same frame again: nothing to send.
    pool.clear();
    for i in 0..300_000 {
        pool.add_instance(instance(i));
    }
    assert!(!pool.reserve(&device));
    pool.upload(&queue);
    assert_eq!(pool.chunks_written(), 0);
    // One person moved: one chunk.
    pool.clear();
    for i in 0..300_000 {
        pool.add_instance(instance(if i == 150_000 { 7 } else { i }));
    }
    pool.upload(&queue);
    assert_eq!(pool.chunks_written(), 1);
    device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
}
