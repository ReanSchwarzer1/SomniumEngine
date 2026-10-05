//! Numerical regression for practical-light bounce. Executes production WGSL
//! with a controlled visibility predicate; native A/Bs exercise the actual TLAS.
#[allow(dead_code)]
#[path = "support/gpu.rs"]
mod gpu;

fn item(source: &str, prefix: &str) -> String {
    let start = source.find(prefix).unwrap();
    let body = start + source[start..].find('{').unwrap();
    let mut depth = 0;
    for (offset, c) in source[body..].char_indices() {
        match c {
            '{' => depth += 1,
            '}' => {
                depth -= 1;
                if depth == 0 { return source[start..body + offset + 1].into(); }
            }
            _ => {}
        }
    }
    panic!("unclosed {prefix}");
}

// Independent numerical surface integral, fixed 800 lm, 2 m below emitter.
fn quadrature(disc: bool) -> f64 {
    let mut total = 0.0;
    let steps = 512;
    let (hx, hy) = (0.8, if disc { 0.8 } else { 0.5 });
    for x in 0..steps {
        for y in 0..steps {
            let px = -hx + (x as f64 + 0.5) * 2.0 * hx / steps as f64;
            let py = -hy + (y as f64 + 0.5) * 2.0 * hy / steps as f64;
            if disc && (0..8).any(|k| {
                let a = (k as f64 + 0.5) * std::f64::consts::FRAC_PI_4;
                px * a.cos() + py * a.sin() > hx * (std::f64::consts::PI / 8.0).cos()
            }) { continue; }
            let r2 = px * px + py * py + 4.0;
            total += 4.0 / (r2 * r2) * (2.0 * hx / steps as f64) * (2.0 * hy / steps as f64);
        }
    }
    let area = if disc { 2.0 * 2.0_f64.sqrt() * hx * hx } else { 4.0 * hx * hy };
    total * 800.0 / (std::f64::consts::PI * area)
}

#[test]
#[ignore = "Requires a graphics adapter; run for local GI changes"]
fn practical_bounce_has_correct_flux_selection_and_sun_off_behavior() {
    let production = include_str!("../src/shaders/restir_gi.wgsl");
    let pool = include_str!("../src/shaders/global_pool.wgsl");
    let mut source = item(pool, "struct GpuLocalLight {");
    source += &item(production, "struct GiLightSample {");
    source += production.lines().find(|line| line.starts_with("const GI_LIGHT_WALK")).unwrap();
    for name in ["gi_rand", "gi_luma", "gi_sample_local", "gi_local_irradiance", "gi_direct_at"] {
        source += &item(production, &format!("fn {name}("));
    }
    source += r#"
        struct Counts { num_local_lights: u32 }
        struct Sun { direction: vec3f, color: vec3f, ibl_intensity: f32 }
        var<private> cluster_params: Counts;
        var<private> local_lights: array<GpuLocalLight,256>;
        var<private> light: Sun;
        var<private> visible: bool;
        const rt_gi_deferred_terrain_albedo = false;
        fn rt_terrain_albedo(i: u32, p: vec3f) -> vec3f { return vec3f(0.5); }
        fn gi_visible(p: vec3f, q: vec3f, t: f32) -> bool { return visible; }
        @group(0) @binding(0) var<storage,read_write> result: array<f32>;
        @compute @workgroup_size(64) fn main(@builtin(global_invocation_id) gid: vec3u) {
            let base = gid.x * 11u;
            let uv = (vec2f(f32(gid.x % 256u), f32(gid.x / 256u)) + 0.5) / 256.0;
            var lamp = GpuLocalLight(vec3f(0,2,0),1000000.0,vec3f(800.0/12.5663706),2u,
                vec3f(0,-1,0),0.8,0.95,0.8,0.8,0.5);
            result[base] = gi_sample_local(lamp,vec3f(0),vec3f(0,1,0),uv).irradiance.x;
            lamp.light_type = 3u;
            result[base+1u] = gi_sample_local(lamp,vec3f(0),vec3f(0,1,0),uv).irradiance.x;
            result[base+2u] = gi_sample_local(lamp,vec3f(0,4,0),vec3f(0,-1,0),uv).irradiance.x;
            result[base+3u] = gi_sample_local(lamp,vec3f(0),vec3f(0,-1,0),uv).irradiance.x;
            lamp.light_type = 1u;
            result[base+4u] = gi_sample_local(lamp,vec3f(0),vec3f(0,1,0),uv).irradiance.x;
            lamp.direction_ws = vec3f(1,0,0);
            result[base+5u] = gi_sample_local(lamp,vec3f(0),vec3f(0,1,0),uv).irradiance.x;
            lamp.light_type = 0u;
            visible = true;
            cluster_params.num_local_lights = 16u;
            for(var i=0u;i<16u;i++) {
                local_lights[i] = lamp;
                local_lights[i].color = vec3f(0);
            }
            local_lights[1].color = vec3f(20,0,0);
            local_lights[11].color = vec3f(40,0,0);
            var seed = gid.x*9781u+17u;
            result[base+6u] = gi_local_irradiance(vec3f(0),vec3f(0,1,0),&seed).x;
            cluster_params.num_local_lights = 1u;
            local_lights[0] = lamp;
            light = Sun(vec3f(0,1,0),vec3f(0),0.0);
            result[base+7u] = gi_direct_at(vec3f(0),vec3f(0,1,0),vec3f(0.5),-1,&seed).x;
            visible = false;
            result[base+8u] = gi_direct_at(vec3f(0),vec3f(0,1,0),vec3f(0.5),-1,&seed).x;
            // More lights than one walk covers: every other one is visited.
            visible = true;
            cluster_params.num_local_lights = 256u;
            for(var i=0u;i<256u;i++) {
                local_lights[i] = lamp;
                local_lights[i].color = vec3f(0);
            }
            local_lights[1].color = vec3f(20,0,0);
            local_lights[12].color = vec3f(40,0,0);
            result[base+10u] = gi_local_irradiance(vec3f(0),vec3f(0,1,0),&seed).x;
            lamp.range = 1.0;
            result[base+9u] = gi_sample_local(lamp,vec3f(0),vec3f(0,1,0),uv).irradiance.x;
        }
    "#;
    let (device, queue) = gpu::device();
    let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("Production practical GI"), source: wgpu::ShaderSource::Wgsl(source.into()),
    });
    let pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
        label: None, layout: None, module: &shader, entry_point: Some("main"),
        compilation_options: Default::default(), cache: None,
    });
    let size = 65536 * 11 * 4;
    let output = device.create_buffer(&wgpu::BufferDescriptor {
        label: None, size, usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC, mapped_at_creation: false,
    });
    let readback = device.create_buffer(&wgpu::BufferDescriptor {
        label: None, size, usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false,
    });
    let bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: None, layout: &pipeline.get_bind_group_layout(0),
        entries: &[wgpu::BindGroupEntry { binding: 0, resource: output.as_entire_binding() }],
    });
    let mut encoder = device.create_command_encoder(&Default::default());
    {
        let mut pass = encoder.begin_compute_pass(&Default::default());
        pass.set_pipeline(&pipeline); pass.set_bind_group(0, &bind, &[]); pass.dispatch_workgroups(1024, 1, 1);
    }
    encoder.copy_buffer_to_buffer(&output, 0, &readback, 0, size);
    queue.submit([encoder.finish()]);
    let (send, recv) = std::sync::mpsc::channel();
    readback.slice(..).map_async(wgpu::MapMode::Read, move |r| send.send(r).unwrap());
    device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
    recv.recv().unwrap().unwrap();
    let mapped = readback.slice(..).get_mapped_range().unwrap();
    let values: &[f32] = bytemuck::cast_slice(&mapped);
    let mut means = [0.0f64; 11];
    let mut brightest_pick = 0.0f32;
    for row in values.chunks_exact(11) {
        for (mean, value) in means.iter_mut().zip(row) { *mean += f64::from(*value) / 65536.0; }
        brightest_pick = brightest_pick.max(row[6]);
    }
    // Two of sixteen lights reach the hit. No single bounce may come back
    // brighter than both together: drawing four of the sixteen at random and
    // scaling by 16/4 returned up to 160 here, the interior levels' white specks.
    assert!(brightest_pick < 15.01, "a bounce returned {brightest_pick} for 15 of light");
    let cd = 800.0 / (4.0 * std::f64::consts::PI);
    let expected = [quadrature(false), quadrature(true), 0.0, 0.0, cd/4.0, 0.0, 15.0, cd/4.0*0.5/std::f64::consts::PI, 0.0, 0.0, 15.0];
    for (i, (actual, expected)) in means.iter().zip(expected).enumerate() {
        let tolerance = if i == 10 { 0.30 } else { 0.025 };
        assert!((actual-expected).abs() < tolerance, "case {i}: {actual} != {expected}");
    }
}

#[test]
#[ignore = "Requires hardware ray queries; run for practical shadow changes"]
fn practical_shadow_respects_switch_and_caster_policy_without_hiding_gi_geometry() {
    use wgpu::util::DeviceExt;
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
    let adapter = gpu::block_on(instance.request_adapter(&Default::default())).unwrap();
    let (device, queue) = gpu::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
        required_features: wgpu::Features::EXPERIMENTAL_RAY_QUERY,
        required_limits: adapter.limits(),
        // Same explicit experimental-API acknowledgement as the renderer.
        experimental_features: unsafe { wgpu::ExperimentalFeatures::enabled() },
        ..Default::default()
    })).unwrap();
    let vertices: [[f32; 8]; 4] = [
        [-0.5,1.0,-0.5,0.0,1.0,0.0,0.0,0.0],
        [0.5,1.0,-0.5,0.0,1.0,0.0,1.0,0.0],
        [0.5,1.0,0.5,0.0,1.0,0.0,1.0,1.0],
        [-0.5,1.0,0.5,0.0,1.0,0.0,0.0,1.0],
    ];
    let vertex_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: None, contents: bytemuck::cast_slice(&vertices), usage: wgpu::BufferUsages::BLAS_INPUT,
    });
    let index_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: None, contents: bytemuck::cast_slice(&[0u32,1,2,0,2,3]), usage: wgpu::BufferUsages::BLAS_INPUT,
    });
    let mut scene = somnium_renderer::pass::raytrace::RaytracePass::new(&device,true);
    scene.register_mesh(&device,0,4,0,6);
    let function = item(include_str!("../src/shaders/shading.wgsl"),"fn practical_visibility(");
    let source = format!(r#"
        enable wgpu_ray_query;
        @group(0) @binding(0) var local_shadow_accel: acceleration_structure;
        @group(0) @binding(1) var<storage,read_write> output_values: array<f32>;
        struct Mode {{ shading_mode: u32 }}
        var<private> cluster_params: Mode;
        {function}
        @compute @workgroup_size(1) fn main() {{
            cluster_params.shading_mode = 16u;
            output_values[0] = practical_visibility(vec3f(0),vec3f(0,1,0),vec3f(0,2,0));
            output_values[1] = practical_visibility(vec3f(2,0,0),vec3f(0,1,0),vec3f(2,2,0));
            output_values[2] = practical_visibility(vec3f(0),vec3f(0,1,0),vec3f(0,0.5,0));
            cluster_params.shading_mode = 0u;
            output_values[3] = practical_visibility(vec3f(0),vec3f(0,1,0),vec3f(0,2,0));
            // GI and reflection intersections must still see a non-caster.
            var rq: ray_query;
            rayQueryInitialize(&rq, local_shadow_accel,
                RayDesc(0u, 0xffu, 0.008, 2.0, vec3f(0), vec3f(0,1,0)));
            rayQueryProceed(&rq);
            output_values[4] = select(0.0, 1.0,
                rayQueryGetCommittedIntersection(&rq).kind != RAY_QUERY_INTERSECTION_NONE);
        }}
    "#);
    let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: None, source: wgpu::ShaderSource::Wgsl(source.into()),
    });
    let pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
        label: None, layout: None, module: &shader, entry_point: Some("main"), compilation_options: Default::default(), cache: None,
    });
    let output = device.create_buffer(&wgpu::BufferDescriptor {
        label: None, size: 20, usage: wgpu::BufferUsages::STORAGE|wgpu::BufferUsages::COPY_SRC, mapped_at_creation: false,
    });
    let readback = device.create_buffer(&wgpu::BufferDescriptor {
        label: None, size: 20, usage: wgpu::BufferUsages::MAP_READ|wgpu::BufferUsages::COPY_DST, mapped_at_creation: false,
    });
    let bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: None, layout: &pipeline.get_bind_group_layout(0), entries: &[
            wgpu::BindGroupEntry { binding:0,resource:wgpu::BindingResource::AccelerationStructure(scene.tlas().unwrap()) },
            wgpu::BindGroupEntry { binding:1,resource:output.as_entire_binding() },
        ],
    });
    // Only the authored flag changes: this also catches a TLAS cache signature
    // that accidentally omits shadow participation and reuses the old mask.
    for casts_shadow in [true, false, true] {
        let mut encoder = device.create_command_encoder(&Default::default());
        scene.build(&device, &mut encoder, &vertex_buffer, &index_buffer,
            &[(0, 0, glam::Mat4::IDENTITY, casts_shadow)]);
        assert_eq!(scene.instance_count(), 1, "non-casters must remain in the TLAS");
        {
            let mut pass=encoder.begin_compute_pass(&Default::default());
            pass.set_pipeline(&pipeline);pass.set_bind_group(0,&bind,&[]);pass.dispatch_workgroups(1,1,1);
        }
        encoder.copy_buffer_to_buffer(&output,0,&readback,0,20);queue.submit([encoder.finish()]);
        let (send,recv)=std::sync::mpsc::channel();
        readback.slice(..).map_async(wgpu::MapMode::Read,move |r|send.send(r).unwrap());
        device.poll(wgpu::PollType::wait_indefinitely()).unwrap();recv.recv().unwrap().unwrap();
        let mapped=readback.slice(..).get_mapped_range().unwrap();
        let actual: &[f32]=bytemuck::cast_slice(&mapped);
        let blocked = if casts_shadow { 0.0 } else { 1.0 };
        assert_eq!(actual,&[blocked,1.0,1.0,1.0,1.0], "casts_shadow={casts_shadow}");
        drop(mapped);
        readback.unmap();
    }
}
