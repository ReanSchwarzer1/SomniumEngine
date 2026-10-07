//! The shared selection/postprocess mapping must preserve the pre-extraction picture.
#[allow(dead_code)]
#[path = "support/gpu.rs"]
mod gpu;

#[test]
#[ignore = "Requires a graphics adapter; run when the shared dream mapping changes"]
fn extracted_mapping_matches_the_frozen_picture_mapping() {
    let (device, queue) = gpu::device();
    let mut source = format!(
        "{}\n{}\n@group(0) @binding(0) var<storage,read_write> differences: array<vec4<f32>>;\n@compute @workgroup_size(1) fn main() {{\n",
        include_str!("../src/shaders/dream_lens.wgsl"),
        include_str!("fixtures/dream_lens_20261006.wgsl")
    );
    let mut count = 0;
    for mode in 0..5 {
        for strength in [0.0, 0.4, 1.0] {
            for (uv, frame, time) in [
                ([0.5, 0.5], [1920.0, 1080.0], 0.0),
                ([0.05, 0.93], [1600.0, 2560.0], 3.1),
                ([0.86, 0.13], [2560.0, 1600.0], 17.29),
            ] {
                let args = format!(
                    "vec2<f32>({},{}),vec4<f32>({mode}.0,{strength},0.85,0.0),{time},vec2<f32>({},{})",
                    uv[0], uv[1], frame[0], frame[1]
                );
                source += &format!(
                    "let a{count}=dream_coordinates({args}); let b{count}=legacy_coordinates({args}); differences[{count}]=vec4<f32>(a{count}.uv-b{count}.uv,a{count}.fracture-b{count}.fracture,a{count}.crack-b{count}.crack);\n"
                );
                count += 1;
            }
        }
    }
    source += "}";
    let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("Dream mapping equivalence"),
        source: wgpu::ShaderSource::Wgsl(source.into()),
    });
    let pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
        label: None,
        layout: None,
        module: &shader,
        entry_point: Some("main"),
        compilation_options: Default::default(),
        cache: None,
    });
    let size = count * 16;
    let output = device.create_buffer(&wgpu::BufferDescriptor {
        label: None,
        size,
        usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
        mapped_at_creation: false,
    });
    let readback = device.create_buffer(&wgpu::BufferDescriptor {
        label: None,
        size,
        usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    let binding = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: None,
        layout: &pipeline.get_bind_group_layout(0),
        entries: &[wgpu::BindGroupEntry {
            binding: 0,
            resource: output.as_entire_binding(),
        }],
    });
    let mut encoder = device.create_command_encoder(&Default::default());
    {
        let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
            label: None,
            timestamp_writes: None,
        });
        pass.set_pipeline(&pipeline);
        pass.set_bind_group(0, &binding, &[]);
        pass.dispatch_workgroups(1, 1, 1);
    }
    encoder.copy_buffer_to_buffer(&output, 0, &readback, 0, size);
    queue.submit([encoder.finish()]);
    let (send, recv) = std::sync::mpsc::channel();
    readback
        .slice(..)
        .map_async(wgpu::MapMode::Read, move |r| send.send(r).unwrap());
    device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
    recv.recv().unwrap().unwrap();
    let bytes = readback.slice(..).get_mapped_range().unwrap();
    let values: &[f32] = bytemuck::cast_slice(&bytes);
    assert!(
        values.iter().all(|v| v.is_finite() && v.abs() < 1e-6),
        "extraction changed the scene mapping: {values:?}"
    );
}
