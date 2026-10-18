//! The batch dispatches on a real device: what `expand` and `shadow_cull` write, read back.
use super::*;
use crate::culling::GpuCullAabb;
use crate::indirect::DrawIndirectArgs;
use crate::instance::GpuInstanceData;

#[allow(dead_code)]
#[path = "../tests/support/gpu.rs"]
mod gpu;

fn buffer(device: &wgpu::Device, size: u64) -> wgpu::Buffer {
    device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("batch test"),
        size,
        usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    })
}

fn read<T: Pod>(device: &wgpu::Device, queue: &wgpu::Queue, source: &wgpu::Buffer) -> Vec<T> {
    let staging = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("batch test readback"),
        size: source.size(),
        usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    let mut encoder = device.create_command_encoder(&Default::default());
    encoder.copy_buffer_to_buffer(source, 0, &staging, 0, source.size());
    queue.submit([encoder.finish()]);
    staging.slice(..).map_async(wgpu::MapMode::Read, |_| {});
    device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
    let bytes = staging.slice(..).get_mapped_range().unwrap().to_vec();
    bytemuck::pod_collect_to_vec(&bytes)
}

fn part(index_count: u32, far: f32) -> BatchPart {
    BatchPart {
        vertex_offset: 100,
        index_offset: 200,
        index_count,
        material_id: 3,
        local: Mat4::IDENTITY,
        near: 0.0,
        far,
        casts_shadow: true,
        shadow_far: 0.0,
    }
}

fn slots(part: &BatchPart, xs: &[f32]) -> Vec<GpuSlot> {
    xs.iter()
        .map(|&x| GpuSlot::new(part, Mat4::from_translation(glam::Vec3::new(x, 0.0, 0.0)), Some(([-1.0; 3], [1.0; 3]))))
        .collect()
}

#[test]
fn expand_writes_arguments_instances_and_bounds_and_a_removed_batch_draws_nothing() {
    let (device, queue) = gpu::device();
    let shaders = crate::shaders::Shaders::new();
    let mut batches = InstanceBatches::default();
    batches.set(1, slots(&part(36, 0.0), &[0.0, 5.0, 10.0]), Vec::new());
    batches.set(2, slots(&part(72, 40.0), &[20.0, 30.0]), Vec::new());
    batches.flush(&device, &queue, &shaders);
    (batches.instance_base, batches.arg_base) = (16, 8);
    let instances = buffer(&device, 32 * std::mem::size_of::<GpuInstanceData>() as u64);
    let args = buffer(&device, 16 * crate::indirect::ARGS_SIZE);
    let aabbs = buffer(&device, 16 * std::mem::size_of::<GpuCullAabb>() as u64);
    let mut encoder = device.create_command_encoder(&Default::default());
    batches.record_expand(&device, &queue, &mut encoder, &instances, &args, &aabbs, false);
    queue.submit([encoder.finish()]);

    let written: Vec<DrawIndirectArgs> = read(&device, &queue, &args);
    assert_eq!(written[..8], [DrawIndirectArgs::default(); 8], "below the region: untouched");
    for (j, count) in [36, 36, 36, 72, 72].into_iter().enumerate() {
        assert_eq!(
            written[8 + j],
            DrawIndirectArgs { vertex_count: count, instance_count: 1, first_vertex: 0, first_instance: 16 + j as u32 }
        );
    }
    let placed: Vec<GpuInstanceData> = read(&device, &queue, &instances);
    assert_eq!(placed[16 + 4].model_matrix[3][0], 30.0);
    assert_eq!((placed[16].material_id, placed[16].mesh_vertex_offset, placed[16].mesh_index_offset), (3, 100, 200));
    let bounds: Vec<GpuCullAabb> = read(&device, &queue, &aabbs);
    assert_eq!(bounds[8 + 3].min, [-1.0, -1.0, -1.0, 40.0], "the far distance rides in min.w");
    assert_eq!(bounds[8].cone[3], 2.0, "no cone test: batches draw two-sided");

    // Batch 1 gone: its slots draw nothing; batch 2's are where they were.
    assert!(batches.remove(1));
    batches.flush(&device, &queue, &shaders);
    let mut encoder = device.create_command_encoder(&Default::default());
    batches.record_expand(&device, &queue, &mut encoder, &instances, &args, &aabbs, false);
    queue.submit([encoder.finish()]);
    let written: Vec<DrawIndirectArgs> = read(&device, &queue, &args);
    assert_eq!(written[8..11], [DrawIndirectArgs::default(); 3]);
    assert_eq!(written[11].vertex_count, 72);
    let bounds: Vec<GpuCullAabb> = read(&device, &queue, &aabbs);
    assert!(bounds[8].min[0] > bounds[8].max[0], "a free slot's bounds are inverted: never culled in");
}

#[test]
fn shadow_cull_keeps_the_casters_inside_each_redrawn_cascade() {
    let (device, queue) = gpu::device();
    let shaders = crate::shaders::Shaders::new();
    let mut batches = InstanceBatches::default();
    // x = 0 and 5 lie in a 20 m box round the origin; x = 50 does not; the fourth does not cast.
    let mut list = slots(&part(36, 0.0), &[0.0, 5.0, 50.0]);
    let mut quiet = part(36, 0.0);
    quiet.casts_shadow = false;
    list.extend(slots(&quiet, &[1.0]));
    batches.set(1, list, Vec::new());
    batches.flush(&device, &queue, &shaders);
    batches.instance_base = 64;
    let box_planes: [[f32; 4]; 6] = [
        [1.0, 0.0, 0.0, 10.0],
        [-1.0, 0.0, 0.0, 10.0],
        [0.0, 1.0, 0.0, 10.0],
        [0.0, -1.0, 0.0, 10.0],
        [0.0, 0.0, 1.0, 10.0],
        [0.0, 0.0, -1.0, 10.0],
    ];
    let planes = [box_planes; crate::shadow::NUM_CASCADES];
    for compact in [true, false] {
        let mut encoder = device.create_command_encoder(&Default::default());
        batches.record_shadow_cull(
            &device,
            &queue,
            &mut encoder,
            &planes,
            glam::Vec3::ZERO,
            [true, false, true, false],
            compact,
            0.0,
        );
        queue.submit([encoder.finish()]);
        let draws = batches.shadow_draws().expect("the cull ran");
        let capacity = draws.capacity as usize;
        let args: Vec<DrawIndirectArgs> = read(&device, &queue, &batches.gpu.as_ref().unwrap().shadow_args);
        if compact {
            let counts: Vec<u32> = read(&device, &queue, draws.counts.unwrap());
            assert_eq!(counts, [2, 0, 2, 0], "two casters in each redrawn cascade, none counted in the others");
            let mut firsts: Vec<u32> = args[2 * capacity..2 * capacity + 2].iter().map(|a| a.first_instance).collect();
            firsts.sort_unstable();
            assert_eq!(firsts, [64, 65]);
        } else {
            assert!(draws.counts.is_none());
            let counts: Vec<u32> = args[..4].iter().map(|a| a.instance_count).collect();
            assert_eq!(counts, [1, 1, 0, 0], "a 0 or 1 count per slot");
        }
    }
}

#[test]
fn a_copy_too_small_at_its_distance_casts_no_shadow() {
    let (device, queue) = gpu::device();
    let shaders = crate::shaders::Shaders::new();
    let mut batches = InstanceBatches::default();
    // Unit boxes (radius ~1.7) 2 m and 8 m from the eye; a threshold of 0.5 keeps only the nearer.
    batches.set(1, slots(&part(36, 0.0), &[2.0, 8.0]), Vec::new());
    batches.flush(&device, &queue, &shaders);
    let open: [[f32; 4]; 6] = [[0.0, 0.0, 0.0, 1.0]; 6];
    let mut encoder = device.create_command_encoder(&Default::default());
    batches.record_shadow_cull(
        &device,
        &queue,
        &mut encoder,
        &[open; crate::shadow::NUM_CASCADES],
        glam::Vec3::ZERO,
        [true, false, false, false],
        true,
        0.5,
    );
    queue.submit([encoder.finish()]);
    let counts: Vec<u32> = read(&device, &queue, batches.shadow_draws().unwrap().counts.unwrap());
    assert_eq!(counts[0], 1);
}
