//! How much CPU a large bindless texture array costs at `CommandEncoder::finish`
//! (12 October 2026: the Town editor spent 6.85 ms a frame in `finish`, with the
//! global group's 2,048-view array bound about 40 times a frame).
//!
//! `cargo test --release -p somnium_renderer --test bindless_finish_cost -- --ignored --nocapture`
#[allow(dead_code)]
#[path = "support/gpu.rs"]
mod gpu;

#[test]
#[ignore = "Requires a bindless graphics adapter; a timing probe, not a check"]
fn finish_cost_by_array_length() {
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
    let adapter = gpu::block_on(instance.request_adapter(&Default::default())).unwrap();
    let (device, _queue) = gpu::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
        required_features: wgpu::Features::TEXTURE_BINDING_ARRAY
            | wgpu::Features::SAMPLED_TEXTURE_AND_STORAGE_BUFFER_ARRAY_NON_UNIFORM_INDEXING
            | wgpu::Features::PARTIALLY_BOUND_BINDING_ARRAY,
        required_limits: adapter.limits(),
        ..Default::default()
    }))
    .unwrap();
    const LAYOUT_COUNT: u32 = 2048;
    let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: None,
        entries: &[wgpu::BindGroupLayoutEntry {
            binding: 0,
            visibility: wgpu::ShaderStages::COMPUTE,
            ty: wgpu::BindingType::Texture {
                sample_type: wgpu::TextureSampleType::Float { filterable: true },
                view_dimension: wgpu::TextureViewDimension::D2,
                multisampled: false,
            },
            count: std::num::NonZeroU32::new(LAYOUT_COUNT),
        }],
    });
    let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: None,
        source: wgpu::ShaderSource::Wgsl(
            "enable wgpu_binding_array;
             @group(0) @binding(0) var textures: binding_array<texture_2d<f32>>;
             @compute @workgroup_size(1) fn main() { let s = textureLoad(textures[0], vec2<i32>(0), 0); }"
                .into(),
        ),
    });
    let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: None,
        bind_group_layouts: &[Some(&layout)],
        immediate_size: 0,
    });
    let pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
        label: None,
        layout: Some(&pipeline_layout),
        module: &module,
        entry_point: Some("main"),
        compilation_options: Default::default(),
        cache: None,
    });
    // Distinct textures, as a level's are (one shared dummy would flatter the tracker).
    let views: Vec<wgpu::TextureView> = (0..LAYOUT_COUNT)
        .map(|_| {
            device
                .create_texture(&wgpu::TextureDescriptor {
                    label: None,
                    size: wgpu::Extent3d { width: 4, height: 4, depth_or_array_layers: 1 },
                    mip_level_count: 1,
                    sample_count: 1,
                    dimension: wgpu::TextureDimension::D2,
                    format: wgpu::TextureFormat::Rgba8Unorm,
                    usage: wgpu::TextureUsages::TEXTURE_BINDING,
                    view_formats: &[],
                })
                .create_view(&Default::default())
        })
        .collect();
    for length in [2048usize, 1300, 600, 64] {
        let refs: Vec<&wgpu::TextureView> = views[..length].iter().collect();
        let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: None,
            layout: &layout,
            entries: &[wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureViewArray(&refs) }],
        });
        let mut best = f64::MAX;
        for _ in 0..5 {
            let mut encoder = device.create_command_encoder(&Default::default());
            for _ in 0..40 {
                let mut pass = encoder.begin_compute_pass(&Default::default());
                pass.set_pipeline(&pipeline);
                pass.set_bind_group(0, &group, &[]);
                pass.dispatch_workgroups(1, 1, 1);
            }
            let started = std::time::Instant::now();
            let buffer = encoder.finish();
            best = best.min(started.elapsed().as_secs_f64() * 1000.0);
            drop(buffer);
        }
        println!("array {length:>4} views, 40 passes: finish {best:.2} ms");
    }
}
