use super::*;

#[test]
fn every_pass_validates() {
    for (label, source) in shader_sources() {
        let module = naga::front::wgsl::parse_str(&source)
            .unwrap_or_else(|e| panic!("{label}: {}", e.emit_to_string(&source)));
        naga::valid::Validator::new(naga::valid::ValidationFlags::all(), naga::valid::Capabilities::SHADER_FLOAT16_IN_FLOAT32)
            .validate(&module)
            .unwrap_or_else(|e| panic!("{label}: {}", e.emit_to_string(&source)));
    }
}

fn gpu() -> Option<(wgpu::Device, wgpu::Queue)> {
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
    let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
        power_preference: wgpu::PowerPreference::HighPerformance,
        ..Default::default()
    }))
    .ok()?;
    pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor::default())).ok()
}

const SIZE: u32 = 256;

fn upload(device: &wgpu::Device, queue: &wgpu::Queue, format: wgpu::TextureFormat, view_format: wgpu::TextureFormat,
    bytes_per_texel: u32, data: &[u8]) -> wgpu::TextureView {
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: None,
        size: wgpu::Extent3d { width: SIZE, height: SIZE, depth_or_array_layers: 1 },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[view_format],
    });
    queue.write_texture(texture.as_image_copy(), data, wgpu::TexelCopyBufferLayout {
        offset: 0,
        bytes_per_row: Some(SIZE * bytes_per_texel),
        rows_per_image: None,
    }, texture.size());
    texture.create_view(&wgpu::TextureViewDescriptor { format: Some(view_format), ..Default::default() })
}

/// A white square on black, `x0..x0 + 32` wide, rows 112..144.
fn square(x0: u32) -> Vec<u8> {
    let mut rgba = vec![0_u8; (SIZE * SIZE * 4) as usize];
    for y in 112..144 {
        for x in x0..x0 + 32 {
            let i = ((y * SIZE + x) * 4) as usize;
            rgba[i..i + 4].copy_from_slice(&[255, 255, 255, 255]);
        }
    }
    rgba
}

/// The whole chain on a square that moved 32 pixels right: the generated frame
/// has it 16 pixels right of where it was, from the game's vectors alone.
#[test]
fn a_moving_square_lands_half_way() {
    let Some((device, queue)) = gpu() else {
        eprintln!("no GPU adapter; skipped");
        return;
    };
    let previous = upload(&device, &queue, wgpu::TextureFormat::Rgba8UnormSrgb, wgpu::TextureFormat::Rgba8Unorm, 4,
        &square(96));
    let current = upload(&device, &queue, wgpu::TextureFormat::Rgba8UnormSrgb, wgpu::TextureFormat::Rgba8Unorm, 4,
        &square(128));

    // Motion `previous_uv - current_uv` on the square's pixels now, as f16 pairs.
    let left = half_bits(-32.0 / SIZE as f32);
    let mut motion = vec![0_u8; (SIZE * SIZE * 4) as usize];
    for y in 112..144 {
        for x in 128..160 {
            let i = ((y * SIZE + x) * 4) as usize;
            motion[i..i + 2].copy_from_slice(&left.to_le_bytes());
        }
    }
    let motion = upload(&device, &queue, wgpu::TextureFormat::Rg16Float, wgpu::TextureFormat::Rg16Float, 4, &motion);
    let depth = depth_square(&device, &queue);

    let proj = [[1.0, 0.0, 0.0, 0.0], [0.0, 1.0, 0.0, 0.0], [0.0, 0.0, 1000.0 / (0.1 - 1000.0), -1.0],
        [0.0, 0.0, 0.1 * 1000.0 / (0.1 - 1000.0), 0.0]];
    let mut fi = FrameInterpolator::new(&device, &queue, [SIZE, SIZE], [SIZE, SIZE]);
    // Scene change detection marks the first frames; let its four history bits clear.
    for frame in 0..8 {
        let mut encoder = device.create_command_encoder(&Default::default());
        fi.dispatch(&device, &queue, &mut encoder, &Inputs {
            depth: &depth,
            motion_vectors: &motion,
            motion_vector_scale: [1.0, 1.0],
            previous: &previous,
            current: &current,
            render_size: [SIZE, SIZE],
            device_to_view: device_to_view_from_projection(proj),
            reset: frame == 0,
            debug_flags: 0,
        });
        queue.submit([encoder.finish()]);
    }

    let pixels = read_rgba8(&device, &queue, fi.output_texture());
    let at = |x: u32, y: u32| pixels[((y * SIZE + x) * 4) as usize];
    for x in [114, 120, 128, 136, 141] {
        assert!(at(x, 128) > 230, "inside the half-way square at x {x}: {}", at(x, 128));
    }
    for x in [98, 104, 108, 148, 152, 158] {
        assert!(at(x, 128) < 25, "outside the half-way square at x {x}: {}", at(x, 128));
    }
}

/// A pan: an 8x8-cell noise picture, `shift` pixels to the right.
fn noise(shift: i32) -> Vec<u8> {
    let mut rgba = vec![0_u8; (SIZE * SIZE * 4) as usize];
    for y in 0..SIZE as i32 {
        for x in 0..SIZE as i32 {
            let (cx, cy) = ((x - shift).div_euclid(8), y.div_euclid(8));
            let h = (cx.wrapping_mul(73_856_093) ^ cy.wrapping_mul(19_349_663)) as u32;
            let v = (h.wrapping_mul(2_654_435_761) >> 24) as u8;
            let i = ((y as u32 * SIZE + x as u32) * 4) as usize;
            rgba[i..i + 4].copy_from_slice(&[v, v, v, 255]);
        }
    }
    rgba
}

/// Optical flow alone: no game vectors, the picture panning 16 pixels back and
/// forth each frame. Once scene change detection and the first-frames rule let it
/// run, the generated frame is the pan half way, far closer to it than a blend.
#[test]
fn optical_flow_alone_follows_a_pan() {
    let Some((device, queue)) = gpu() else {
        eprintln!("no GPU adapter; skipped");
        return;
    };
    let a = upload(&device, &queue, wgpu::TextureFormat::Rgba8UnormSrgb, wgpu::TextureFormat::Rgba8Unorm, 4, &noise(0));
    let b = upload(&device, &queue, wgpu::TextureFormat::Rgba8UnormSrgb, wgpu::TextureFormat::Rgba8Unorm, 4,
        &noise(16));
    let still = upload(&device, &queue, wgpu::TextureFormat::Rg16Float, wgpu::TextureFormat::Rg16Float, 4,
        &vec![0_u8; (SIZE * SIZE * 4) as usize]);
    let depth = depth_square(&device, &queue);
    let proj = [[1.0, 0.0, 0.0, 0.0], [0.0, 1.0, 0.0, 0.0], [0.0, 0.0, 1000.0 / (0.1 - 1000.0), -1.0],
        [0.0, 0.0, 0.1 * 1000.0 / (0.1 - 1000.0), 0.0]];
    let mut fi = FrameInterpolator::new(&device, &queue, [SIZE, SIZE], [SIZE, SIZE]);
    for frame in 0..15 {
        let (previous, current) = if frame % 2 == 0 { (&a, &b) } else { (&b, &a) };
        let mut encoder = device.create_command_encoder(&Default::default());
        fi.dispatch(&device, &queue, &mut encoder, &Inputs {
            depth: &depth,
            motion_vectors: &still,
            motion_vector_scale: [1.0, 1.0],
            previous,
            current,
            render_size: [SIZE, SIZE],
            device_to_view: device_to_view_from_projection(proj),
            reset: frame == 0,
            debug_flags: 0,
        });
        queue.submit([encoder.finish()]);
    }
    // Frame 14: shift 0 before, 16 now; half way is 8.
    let out = read_rgba8(&device, &queue, fi.output_texture());
    let (mid, before, now) = (noise(8), noise(0), noise(16));
    let (mut err, mut blend_err, mut n) = (0.0_f64, 0.0_f64, 0.0_f64);
    for y in 32..SIZE - 32 {
        for x in 32..SIZE - 32 {
            let i = ((y * SIZE + x) * 4) as usize;
            let blend = (f64::from(before[i]) + f64::from(now[i])) * 0.5;
            err += (f64::from(out[i]) - f64::from(mid[i])).abs();
            blend_err += (blend - f64::from(mid[i])).abs();
            n += 1.0;
        }
    }
    let (err, blend_err) = (err / n, blend_err / n);
    eprintln!("mean error against the half-way pan {err:.1} (a 50/50 blend: {blend_err:.1})");
    // AMD weighs flow against the (here absent) game vectors by how much each pair of
    // colours agrees, so with no game vectors the result is part blend by design.
    assert!(err < blend_err * 0.7, "flow error {err:.1}, blend {blend_err:.1}");

    // The flow itself: interior blocks point 16 pixels back to where they were.
    let staging = device.create_buffer(&wgpu::BufferDescriptor { label: None, size: 32 * 32 * 8,
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ, mapped_at_creation: false });
    let mut encoder = device.create_command_encoder(&Default::default());
    encoder.copy_buffer_to_buffer(&fi.display.flow_filtered[0], 0, &staging, 0, 32 * 32 * 8);
    queue.submit([encoder.finish()]);
    staging.slice(..).map_async(wgpu::MapMode::Read, |r| r.expect("map"));
    device.poll(wgpu::PollType::wait_indefinitely()).expect("poll");
    let flow: Vec<i32> = bytemuck::cast_slice(&staging.slice(..).get_mapped_range().expect("mapped")).to_vec();
    let interior: Vec<(i32, i32)> = (4..28).flat_map(|y| (4..28).map(move |x| (x, y)))
        .map(|(x, y)| (flow[(y * 32 + x) * 2], flow[(y * 32 + x) * 2 + 1])).collect();
    let right = interior.iter().filter(|v| **v == (-16, 0)).count();
    eprintln!("{right} of {} interior blocks at (-16, 0)", interior.len());
    assert!(right * 10 >= interior.len() * 9, "{right} of {}", interior.len());
}

fn half_bits(v: f32) -> u16 {
    // Exact for the multiples of 1/256 used here.
    let bits = v.to_bits();
    let sign = ((bits >> 16) & 0x8000) as u16;
    let exponent = ((bits >> 23) & 0xff) as i32 - 127 + 15;
    let mantissa = ((bits >> 13) & 0x3ff) as u16;
    if v == 0.0 { 0 } else { sign | ((exponent as u16) << 10) | mantissa }
}

/// Depth 0.5 on the square's pixels now, 0.9 elsewhere (depth formats take no queue writes).
fn depth_square(device: &wgpu::Device, queue: &wgpu::Queue) -> wgpu::TextureView {
    let depth = device.create_texture(&wgpu::TextureDescriptor {
        label: None,
        size: wgpu::Extent3d { width: SIZE, height: SIZE, depth_or_array_layers: 1 },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Depth32Float,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::RENDER_ATTACHMENT,
        view_formats: &[],
    });
    let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: None,
        source: wgpu::ShaderSource::Wgsl(
            "@vertex fn vs(@builtin(vertex_index) i: u32) -> @builtin(position) vec4<f32> {
                 let p = vec2<f32>(f32((i << 1u) & 2u), f32(i & 2u));
                 return vec4<f32>(p * 4.0 - 1.0, 0.0, 1.0);
             }
             @fragment fn fs(@builtin(position) p: vec4<f32>) -> @builtin(frag_depth) f32 {
                 let inside = p.x >= 128.0 && p.x < 160.0 && p.y >= 112.0 && p.y < 144.0;
                 return select(0.9, 0.5, inside);
             }"
            .into(),
        ),
    });
    let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: None,
        layout: None,
        vertex: wgpu::VertexState { module: &module, entry_point: Some("vs"), compilation_options: Default::default(), buffers: &[] },
        primitive: Default::default(),
        depth_stencil: Some(wgpu::DepthStencilState {
            format: wgpu::TextureFormat::Depth32Float,
            depth_write_enabled: Some(true),
            depth_compare: Some(wgpu::CompareFunction::Always),
            stencil: Default::default(),
            bias: Default::default(),
        }),
        multisample: Default::default(),
        fragment: Some(wgpu::FragmentState { module: &module, entry_point: Some("fs"), compilation_options: Default::default(), targets: &[] }),
        multiview_mask: None,
        cache: None,
    });
    let view = depth.create_view(&Default::default());
    let mut encoder = device.create_command_encoder(&Default::default());
    {
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: None,
            color_attachments: &[],
            depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                view: &view,
                depth_ops: Some(wgpu::Operations { load: wgpu::LoadOp::Clear(1.0), store: wgpu::StoreOp::Store }),
                stencil_ops: None,
            }),
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        pass.set_pipeline(&pipeline);
        pass.draw(0..3, 0..1);
    }
    queue.submit([encoder.finish()]);
    depth.create_view(&wgpu::TextureViewDescriptor { aspect: wgpu::TextureAspect::DepthOnly, ..Default::default() })
}

fn read_rgba8(device: &wgpu::Device, queue: &wgpu::Queue, texture: &wgpu::Texture) -> Vec<u8> {
    let buffer = device.create_buffer(&wgpu::BufferDescriptor {
        label: None,
        size: u64::from(SIZE * SIZE * 4),
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let mut encoder = device.create_command_encoder(&Default::default());
    encoder.copy_texture_to_buffer(texture.as_image_copy(), wgpu::TexelCopyBufferInfo {
        buffer: &buffer,
        layout: wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(SIZE * 4), rows_per_image: None },
    }, wgpu::Extent3d { width: SIZE, height: SIZE, depth_or_array_layers: 1 });
    queue.submit([encoder.finish()]);
    buffer.slice(..).map_async(wgpu::MapMode::Read, |r| r.expect("map"));
    device.poll(wgpu::PollType::wait_indefinitely()).expect("poll");
    let data = buffer.slice(..).get_mapped_range().expect("mapped").to_vec();
    data
}

/// GPU cost of one generated frame at 2560x1440 (render 1715x965, FSR at 67%), by timestamps.
/// `cargo test -p wgpu-ffx-fg --release -- --ignored --nocapture cost`
#[test]
#[ignore]
fn cost_at_1440p() {
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
    let Ok(adapter) = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
        power_preference: wgpu::PowerPreference::HighPerformance,
        ..Default::default()
    })) else {
        return;
    };
    let features = wgpu::Features::TIMESTAMP_QUERY | wgpu::Features::TIMESTAMP_QUERY_INSIDE_ENCODERS;
    let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
        required_features: features,
        ..Default::default()
    }))
    .expect("device");
    let (display, render) = ([2560_u32, 1440], [1715_u32, 965]);
    let texture = |size: [u32; 2], format: wgpu::TextureFormat, usage: wgpu::TextureUsages| {
        device.create_texture(&wgpu::TextureDescriptor {
            label: None,
            size: wgpu::Extent3d { width: size[0], height: size[1], depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | usage,
            view_formats: &[wgpu::TextureFormat::Bgra8Unorm],
        })
    };
    let picture = |_| texture(display, wgpu::TextureFormat::Bgra8UnormSrgb, wgpu::TextureUsages::empty())
        .create_view(&wgpu::TextureViewDescriptor { format: Some(wgpu::TextureFormat::Bgra8Unorm), ..Default::default() });
    let (a, b) = (picture(0), picture(1));
    let motion = device.create_texture(&wgpu::TextureDescriptor {
        label: None,
        size: wgpu::Extent3d { width: render[0], height: render[1], depth_or_array_layers: 1 },
        mip_level_count: 1, sample_count: 1, dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rg16Float, usage: wgpu::TextureUsages::TEXTURE_BINDING, view_formats: &[],
    }).create_view(&Default::default());
    let depth = device.create_texture(&wgpu::TextureDescriptor {
        label: None,
        size: wgpu::Extent3d { width: render[0], height: render[1], depth_or_array_layers: 1 },
        mip_level_count: 1, sample_count: 1, dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Depth32Float, usage: wgpu::TextureUsages::TEXTURE_BINDING, view_formats: &[],
    }).create_view(&wgpu::TextureViewDescriptor { aspect: wgpu::TextureAspect::DepthOnly, ..Default::default() });
    let queries = device.create_query_set(&wgpu::QuerySetDescriptor { label: None, ty: wgpu::QueryType::Timestamp, count: 2 });
    let resolve = device.create_buffer(&wgpu::BufferDescriptor { label: None, size: 16,
        usage: wgpu::BufferUsages::QUERY_RESOLVE | wgpu::BufferUsages::COPY_SRC, mapped_at_creation: false });
    let readback = device.create_buffer(&wgpu::BufferDescriptor { label: None, size: 16,
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ, mapped_at_creation: false });
    let proj = [[1.0, 0.0, 0.0, 0.0], [0.0, 1.0, 0.0, 0.0], [0.0, 0.0, 1000.0 / (0.1 - 1000.0), -1.0],
        [0.0, 0.0, 0.1 * 1000.0 / (0.1 - 1000.0), 0.0]];
    let mut fi = FrameInterpolator::new(&device, &queue, display, render);
    let mut times = Vec::new();
    let mut cpu = Vec::new();
    for frame in 0..60 {
        let started = std::time::Instant::now();
        let mut encoder = device.create_command_encoder(&Default::default());
        encoder.write_timestamp(&queries, 0);
        fi.dispatch(&device, &queue, &mut encoder, &Inputs {
            depth: &depth, motion_vectors: &motion, motion_vector_scale: [1.0, 1.0],
            previous: if frame % 2 == 0 { &a } else { &b }, current: if frame % 2 == 0 { &b } else { &a },
            render_size: render, device_to_view: device_to_view_from_projection(proj), reset: false, debug_flags: 0,
        });
        encoder.write_timestamp(&queries, 1);
        encoder.resolve_query_set(&queries, 0..2, &resolve, 0);
        encoder.copy_buffer_to_buffer(&resolve, 0, &readback, 0, 16);
        let commands = encoder.finish();
        cpu.push(started.elapsed().as_secs_f64() * 1000.0);
        queue.submit([commands]);
        readback.slice(..).map_async(wgpu::MapMode::Read, |r| r.expect("map"));
        device.poll(wgpu::PollType::wait_indefinitely()).expect("poll");
        let t: [u64; 2] = bytemuck::pod_read_unaligned(&readback.slice(..).get_mapped_range().expect("mapped")[..16]);
        readback.unmap();
        if frame >= 10 {
            times.push((t[1] - t[0]) as f64 * f64::from(queue.get_timestamp_period()) / 1.0e6);
        }
    }
    times.sort_by(f64::total_cmp);
    cpu.sort_by(f64::total_cmp);
    eprintln!("frame generation at 2560x1440 (render 1715x965): GPU median {:.3} ms, min {:.3}, max {:.3}; CPU record median {:.3} ms",
        times[times.len() / 2], times[0], times[times.len() - 1], cpu[cpu.len() / 2]);
}

/// Review: a near dark post moving 128 px over a static textured background (the case the Town stairs showed
/// speckled). Prints speckle counts; with `FG_REVIEW_DIR` set, writes previous / generated / current as PPM.
/// `FG_REVIEW_DIR=... cargo test -p wgpu-ffx-fg -- --ignored --nocapture fast_post`
#[test]
#[ignore]
fn fast_post_review() {
    let Some((device, queue)) = gpu() else { return };
    const W: u32 = 512;
    const H: u32 = 256;
    let scale: f32 = std::env::var("FG_REVIEW_SCALE").ok().and_then(|s| s.parse().ok()).unwrap_or(0.67);
    let (rw, rh) = ((W as f32 * scale) as u32, (H as f32 * scale) as u32);
    let tex = |format: wgpu::TextureFormat, view: wgpu::TextureFormat, data: &[u8], (w, h): (u32, u32)| {
        let t = device.create_texture(&wgpu::TextureDescriptor {
            label: None,
            size: wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[view],
        });
        queue.write_texture(t.as_image_copy(), data, wgpu::TexelCopyBufferLayout {
            offset: 0,
            bytes_per_row: Some(w * 4),
            rows_per_image: None,
        }, t.size());
        t
    };
    let picture = |x0: u32| {
        let mut rgba = vec![0_u8; (W * H * 4) as usize];
        for y in 0..H {
            for x in 0..W {
                let h = ((x / 4).wrapping_mul(73_856_093) ^ (y / 4).wrapping_mul(19_349_663)).wrapping_mul(2_654_435_761) >> 24;
                let v = if x >= x0 && x < x0 + 60 { 12 } else { 120 + (h as u8 / 2) };
                let i = ((y * W + x) * 4) as usize;
                rgba[i..i + 4].copy_from_slice(&[v, v, v, 255]);
            }
        }
        rgba
    };
    let (prev_x, cur_x) = (300_u32, 172_u32);
    let raw = |t: &wgpu::Texture| t.create_view(&wgpu::TextureViewDescriptor {
        format: Some(wgpu::TextureFormat::Rgba8Unorm),
        ..Default::default()
    });
    let prev = tex(wgpu::TextureFormat::Rgba8UnormSrgb, wgpu::TextureFormat::Rgba8Unorm, &picture(prev_x), (W, H));
    let cur = tex(wgpu::TextureFormat::Rgba8UnormSrgb, wgpu::TextureFormat::Rgba8Unorm, &picture(cur_x), (W, H));
    let (prev_v, cur_v) = (raw(&prev), raw(&cur));
    let mv_bits = half_bits((prev_x as f32 - cur_x as f32) / W as f32);
    let (post_r0, post_r1) = ((cur_x as f32 * scale) as u32, ((cur_x + 60) as f32 * scale) as u32);
    let mut motion = vec![0_u8; (rw * rh * 4) as usize];
    for y in 0..rh {
        for x in post_r0..post_r1 {
            let i = ((y * rw + x) * 4) as usize;
            motion[i..i + 2].copy_from_slice(&mv_bits.to_le_bytes());
        }
    }
    let motion = tex(wgpu::TextureFormat::Rg16Float, wgpu::TextureFormat::Rg16Float, &motion, (rw, rh))
        .create_view(&Default::default());
    let depth = device.create_texture(&wgpu::TextureDescriptor {
        label: None,
        size: wgpu::Extent3d { width: rw, height: rh, depth_or_array_layers: 1 },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Depth32Float,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::RENDER_ATTACHMENT,
        view_formats: &[],
    });
    {
        let source = format!(
            "@vertex fn vs(@builtin(vertex_index) i: u32) -> @builtin(position) vec4<f32> {{
                 let p = vec2<f32>(f32((i << 1u) & 2u), f32(i & 2u));
                 return vec4<f32>(p * 4.0 - 1.0, 0.0, 1.0);
             }}
             @fragment fn fs(@builtin(position) p: vec4<f32>) -> @builtin(frag_depth) f32 {{
                 return select(0.95, 0.2, p.x >= {}.0 && p.x < {}.0);
             }}",
            post_r0,
            post_r1
        );
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: None,
            source: wgpu::ShaderSource::Wgsl(source.into()),
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: None,
            layout: None,
            vertex: wgpu::VertexState { module: &module, entry_point: Some("vs"), compilation_options: Default::default(), buffers: &[] },
            primitive: Default::default(),
            depth_stencil: Some(wgpu::DepthStencilState {
                format: wgpu::TextureFormat::Depth32Float,
                depth_write_enabled: Some(true),
                depth_compare: Some(wgpu::CompareFunction::Always),
                stencil: Default::default(),
                bias: Default::default(),
            }),
            multisample: Default::default(),
            fragment: Some(wgpu::FragmentState { module: &module, entry_point: Some("fs"), compilation_options: Default::default(), targets: &[] }),
            multiview_mask: None,
            cache: None,
        });
        let dv = depth.create_view(&Default::default());
        let mut e = device.create_command_encoder(&Default::default());
        {
            let mut pass = e.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: None,
                color_attachments: &[],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &dv,
                    depth_ops: Some(wgpu::Operations { load: wgpu::LoadOp::Clear(1.0), store: wgpu::StoreOp::Store }),
                    stencil_ops: None,
                }),
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pass.set_pipeline(&pipeline);
            pass.draw(0..3, 0..1);
        }
        queue.submit([e.finish()]);
    }
    let depth = depth.create_view(&wgpu::TextureViewDescriptor { aspect: wgpu::TextureAspect::DepthOnly, ..Default::default() });
    let proj = [[1.0, 0.0, 0.0, 0.0], [0.0, 1.0, 0.0, 0.0], [0.0, 0.0, 1000.0 / (0.1 - 1000.0), -1.0],
        [0.0, 0.0, 0.1 * 1000.0 / (0.1 - 1000.0), 0.0]];
    let mut fi = FrameInterpolator::new(&device, &queue, [W, H], [rw, rh]);
    for frame in 0..8 {
        let mut e = device.create_command_encoder(&Default::default());
        fi.dispatch(&device, &queue, &mut e, &Inputs {
            depth: &depth,
            motion_vectors: &motion,
            motion_vector_scale: [1.0, 1.0],
            previous: &prev_v,
            current: &cur_v,
            render_size: [rw, rh],
            device_to_view: device_to_view_from_projection(proj),
            reset: frame == 0,
            debug_flags: 0,
        });
        queue.submit([e.finish()]);
    }
    let staging = device.create_buffer(&wgpu::BufferDescriptor {
        label: None,
        size: u64::from(W * H * 4),
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let mut e = device.create_command_encoder(&Default::default());
    e.copy_texture_to_buffer(fi.output_texture().as_image_copy(), wgpu::TexelCopyBufferInfo {
        buffer: &staging,
        layout: wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(W * 4), rows_per_image: None },
    }, wgpu::Extent3d { width: W, height: H, depth_or_array_layers: 1 });
    queue.submit([e.finish()]);
    staging.slice(..).map_async(wgpu::MapMode::Read, |r| r.expect("map"));
    device.poll(wgpu::PollType::wait_indefinitely()).expect("poll");
    let out = staging.slice(..).get_mapped_range().expect("mapped").to_vec();
    let mid = (prev_x + cur_x) / 2;
    let (mut bright_in_post, mut n_post, mut dark_outside, mut n_out) = (0, 0, 0, 0);
    for y in 8..H - 8 {
        for x in 8..W - 8 {
            let v = out[((y * W + x) * 4) as usize];
            if x >= mid + 4 && x < mid + 56 {
                n_post += 1;
                if v > 60 {
                    bright_in_post += 1;
                }
            }
            if x + 8 < mid || x > mid + 68 {
                n_out += 1;
                if v < 60 {
                    dark_outside += 1;
                }
            }
        }
    }
    eprintln!("post at x {mid}..{}: {bright_in_post}/{n_post} bright pixels inside; {dark_outside}/{n_out} dark outside", mid + 60);
    if let Ok(dir) = std::env::var("FG_REVIEW_DIR") {
        let ppm = |name: &str, rgba: &[u8]| {
            let mut data = format!("P6 {W} {H} 255\n").into_bytes();
            for px in rgba.chunks_exact(4) {
                data.extend_from_slice(&px[..3]);
            }
            std::fs::write(format!("{dir}/{name}.ppm"), data).expect("write");
        };
        ppm("a_previous", &picture(prev_x));
        ppm("b_generated", &out);
        ppm("c_current", &picture(cur_x));
    }
}
