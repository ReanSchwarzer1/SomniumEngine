//! Exercise the production area-light integrals on a graphics adapter.
#[allow(dead_code)]
#[path = "support/gpu.rs"]
mod gpu;

fn production_function(name: &str) -> &'static str {
    let source = include_str!("../src/shaders/shading.wgsl");
    let start = source.find(&format!("fn {name}(")).unwrap();
    let body = start + source[start..].find('{').unwrap();
    let mut depth = 0;
    for (offset, c) in source[body..].char_indices() {
        match c {
            '{' => depth += 1,
            '}' => {
                depth -= 1;
                if depth == 0 {
                    return &source[start..body + offset + 1];
                }
            }
            _ => {}
        }
    }
    panic!("unterminated production function {name}");
}

// Independent area quadrature: cos(receiver) * cos(emitter) / (pi * r^2).
// The production disc approximation is an inscribed regular octagon.
fn reference(octagon: bool, distance: f64, hx: f64, hy: f64) -> f32 {
    let steps = 512;
    let dx = 2.0 * hx / steps as f64;
    let dy = 2.0 * hy / steps as f64;
    let mut sum = 0.0;
    for ix in 0..steps {
        for iy in 0..steps {
            let x = -hx + (ix as f64 + 0.5) * dx;
            let y = -hy + (iy as f64 + 0.5) * dy;
            if octagon
                && (0..8).any(|edge| {
                    let angle = (edge as f64 + 0.5) * std::f64::consts::FRAC_PI_4;
                    x * angle.cos() + y * angle.sin() > hx * (std::f64::consts::PI / 8.0).cos()
                })
            {
                continue;
            }
            let r2 = x * x + y * y + distance * distance;
            sum += distance * distance / (std::f64::consts::PI * r2 * r2) * dx * dy;
        }
    }
    sum as f32
}

#[test]
#[ignore = "Requires a graphics adapter; run for area-light shader changes"]
fn area_emitters_light_the_front_and_not_the_back() {
    let (device, queue) = gpu::device();
    let calls = [
        // Downward ceiling fixture, upward receiver.
        "vec3f(0,0,0), vec3f(0,1,0), vec3f(0,2,0), vec3f(0,-1,0)",
        // Same geometry translated and rotated onto a wall.
        "vec3f(3,4,7), vec3f(0,0,-1), vec3f(3,4,5), vec3f(0,0,1)",
        // Receiver behind the emitter, facing it: must stay dark.
        "vec3f(0,4,0), vec3f(0,-1,0), vec3f(0,2,0), vec3f(0,-1,0)",
        // Front side of emitter but back side of receiver: also dark.
        "vec3f(0,0,0), vec3f(0,-1,0), vec3f(0,2,0), vec3f(0,-1,0)",
    ];
    let mut source = format!(
        "{}\n{}\n{}\n@group(0) @binding(0) var<storage, read_write> result: array<f32>;\n@compute @workgroup_size(1) fn main() {{\n",
        production_function("ltc_quad_diffuse"),
        production_function("ltc_disc_diffuse"),
        production_function("area_irradiance_scale")
    );
    for (i, call) in calls.iter().enumerate() {
        source += &format!("result[{}] = ltc_quad_diffuse({call}, 0.8, 0.5);\n", i * 2);
        source += &format!("result[{}] = ltc_disc_diffuse({call}, 0.8);\n", i * 2 + 1);
    }
    // Keep total flux at 800 lm while changing distance, size and receiver
    // orientation. Compare actual GPU illuminance, not only a geometric term.
    source += r#"
        let cd = 800.0 / 12.5663706;
        let r2 = ltc_quad_diffuse(vec3f(0), vec3f(0,1,0), vec3f(0,2,0), vec3f(0,-1,0), 0.8, 0.5);
        let r4 = ltc_quad_diffuse(vec3f(0), vec3f(0,1,0), vec3f(0,4,0), vec3f(0,-1,0), 0.8, 0.5);
        let small = ltc_quad_diffuse(vec3f(0), vec3f(0,1,0), vec3f(0,2,0), vec3f(0,-1,0), 0.2, 0.125);
        let tilted = ltc_quad_diffuse(vec3f(0), vec3f(0.6,0.8,0), vec3f(0,2,0), vec3f(0,-1,0), 0.8, 0.5);
        let d2 = ltc_disc_diffuse(vec3f(0), vec3f(0,1,0), vec3f(0,2,0), vec3f(0,-1,0), 0.8);
        let d4 = ltc_disc_diffuse(vec3f(0), vec3f(0,1,0), vec3f(0,4,0), vec3f(0,-1,0), 0.8);
        result[8] = cd * area_irradiance_scale(r2, 1.6, 2.0, 1000000.0);
        result[9] = cd * area_irradiance_scale(r4, 1.6, 4.0, 1000000.0);
        result[10] = cd * area_irradiance_scale(small, 0.1, 2.0, 1000000.0);
        result[11] = cd * area_irradiance_scale(tilted, 1.6, 2.0, 1000000.0);
        result[12] = cd * area_irradiance_scale(d2, 2.82842712*0.64, 2.0, 1000000.0);
        result[13] = cd * area_irradiance_scale(d4, 2.82842712*0.64, 4.0, 1000000.0);
        result[14] = cd * area_irradiance_scale(r2, 1.6, 2.0, 4.0);
        result[15] = cd * area_irradiance_scale(r4, 1.6, 4.0, 4.0);
    }"#;
    let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("Production Rect and Disc irradiance"),
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
    let output = device.create_buffer(&wgpu::BufferDescriptor {
        label: None,
        size: 64,
        usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
        mapped_at_creation: false,
    });
    let readback = device.create_buffer(&wgpu::BufferDescriptor {
        label: None,
        size: 64,
        usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    let bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: None,
        layout: &pipeline.get_bind_group_layout(0),
        entries: &[wgpu::BindGroupEntry {
            binding: 0,
            resource: output.as_entire_binding(),
        }],
    });
    let mut encoder = device.create_command_encoder(&Default::default());
    {
        let mut pass = encoder.begin_compute_pass(&Default::default());
        pass.set_pipeline(&pipeline);
        pass.set_bind_group(0, &bind, &[]);
        pass.dispatch_workgroups(1, 1, 1);
    }
    encoder.copy_buffer_to_buffer(&output, 0, &readback, 0, 64);
    queue.submit([encoder.finish()]);
    let (send, recv) = std::sync::mpsc::channel();
    readback
        .slice(..)
        .map_async(wgpu::MapMode::Read, move |r| send.send(r).unwrap());
    device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
    recv.recv().unwrap().unwrap();
    let mapped = readback.slice(..).get_mapped_range().unwrap();
    let actual: &[f32] = bytemuck::cast_slice(&mapped);
    let rect = reference(false, 2.0, 0.8, 0.5);
    let disc = reference(true, 2.0, 0.8, 0.8);
    for (i, expected) in [rect, disc, rect, disc, 0.0, 0.0, 0.0, 0.0]
        .iter()
        .enumerate()
    {
        assert!(
            (actual[i] - expected).abs() < 0.0003,
            "fixture {i}: GPU {} != independent irradiance {expected}",
            actual[i]
        );
    }
    let illuminance = [
        800.0 / 1.6 * rect,
        800.0 / 1.6 * reference(false, 4.0, 0.8, 0.5),
        800.0 / 0.1 * reference(false, 2.0, 0.2, 0.125),
        800.0 / 1.6 * rect * 0.8,
        800.0 / (2.0 * 2.0_f32.sqrt() * 0.64) * disc,
        800.0 / (2.0 * 2.0_f32.sqrt() * 0.64) * reference(true, 4.0, 0.8, 0.8),
        800.0 / 1.6 * rect * (15.0_f32 / 16.0).powi(2),
        0.0,
    ];
    for (i, expected) in illuminance.iter().enumerate() {
        assert!(
            (actual[i + 8] - expected).abs() < 0.04,
            "photometry {i}: GPU {} lux != independent {expected} lux",
            actual[i + 8]
        );
    }
}
