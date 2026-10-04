//! Exercises the production spatial GI stage and its actual rgba16float target.
//! Geometry/visibility are controlled; stochastic neighbour reuse is disabled.
//! SOMNIUM_GI_SHADER_UNDER_TEST can select a saved pre-fix shader for a red run.
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
                if depth == 0 {
                    return source[start..body + offset + 1].into();
                }
            }
            _ => {}
        }
    }
    panic!("unclosed {prefix}");
}

fn half_to_float(bits: u16) -> f32 {
    let sign = if bits & 0x8000 == 0 { 1.0 } else { -1.0 };
    let exponent = (bits >> 10) & 31;
    let mantissa = f32::from(bits & 1023) / 1024.0;
    match exponent {
        0 => sign * mantissa * 2.0_f32.powi(-14),
        31 if mantissa == 0.0 => sign * f32::INFINITY,
        31 => f32::NAN,
        _ => sign * (1.0 + mantissa) * 2.0_f32.powi(i32::from(exponent) - 15),
    }
}

fn fixture() -> (Vec<[f32; 4]>, Vec<f32>) {
    let production = std::env::var("SOMNIUM_GI_SHADER_UNDER_TEST")
        .map(|path| std::fs::read_to_string(path).unwrap())
        .unwrap_or_else(|_| include_str!("../src/shaders/restir_gi.wgsl").into());
    let mut source = String::new();
    for prefix in ["struct GiReservoir {", "struct GiSurface {", "fn gi_empty(",
        "fn gi_grid(", "fn gi_full_coord(", "fn gi_luma(", "fn gi_rand("] {
        source += &item(&production, prefix);
    }
    if production.contains("fn gi_encode_output(") {
        source += &item(&production, "fn gi_encode_output(");
    }
    // Only adapt the entry-point signature so fixture setup and history
    // inspection surround the unmodified production spatial-stage body.
    source += &item(&production, "fn spatial_and_shade(")
        .replace("@builtin(global_invocation_id) ", "");
    source += r#"
        struct Params { scale:f32, frame:u32, history_valid:f32,
            camera_pos:vec3f, intensity:f32 }
        struct Sun { color:vec3f }
        struct Counts { num_local_lights:u32 }
        var<private> gi:Params;
        var<private> light:Sun;
        var<private> cluster_params:Counts;
        const GI_SPATIAL_TAPS=0u;
        const GI_SPATIAL_RADIUS=10.0;
        @group(0) @binding(0) var depth_tex:texture_depth_2d;
        @group(0) @binding(1) var out_tex:texture_storage_2d<rgba16float,write>;
        @group(0) @binding(2) var<storage,read_write> gi_a:array<GiReservoir>;
        @group(0) @binding(3) var<storage,read_write> gi_b:array<GiReservoir>;
        @group(0) @binding(4) var grain_masks:texture_2d_array<f32>;
        @group(0) @binding(5) var<storage,read_write> history:array<f32>;
        fn gi_visible(p:vec3f,q:vec3f,t:f32)->bool { return true; }
        fn gi_spatial_surface(index:u32,coord:vec2i,dims:vec2u)->GiSurface {
            return GiSurface(index!=3u,vec3f(0),vec3f(0,1,0));
        }
        fn gi_merge(r:ptr<function,GiReservoir>,p:vec3f,n:vec3f,
            other:GiReservoir,other_pos:vec3f,seed:ptr<function,u32>) {}
        @compute @workgroup_size(8,1,1)
        fn main(@builtin(global_invocation_id) id:vec3u) {
            gi=Params(1.0,0u,0.0,vec3f(0,0,2),1.0);
            light=Sun(vec3f(1));
            cluster_params=Counts(0u);
            var radiance=vec3f(2,4,8);
            if id.x<2u {
                light.color=vec3f(0);
                cluster_params.num_local_lights=select(0u,1u,id.x==0u);
            }
            // Valid finite transport input, above rgba16float's finite range.
            if id.x==4u { radiance=vec3f(1000000.0); }
            if id.x==5u { radiance=vec3f(bitcast<f32>(0x7fc00000u)); }
            if id.x==6u { radiance=vec3f(bitcast<f32>(0x7f800000u)); }
            if id.x==7u { gi.camera_pos=vec3f(0,0,1000000.0); }
            let reservoir=GiReservoir(vec3f(0,1,0),gi_luma(radiance),
                vec3f(0,-1,0),1.0,radiance,1.0);
            gi_a[id.x]=reservoir;
            gi_b[id.x]=reservoir;
            spatial_and_shade(id);
            history[id.x]=gi_a[id.x].m;
        }
    "#;
    let (device, queue) = gpu::device();
    let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("Production GI spatial output regression"),
        source: wgpu::ShaderSource::Wgsl(source.into()),
    });
    let pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
        label: None, layout: None, module: &module, entry_point: Some("main"),
        compilation_options: Default::default(), cache: None,
    });
    let texture = |format, usage, layers| device.create_texture(&wgpu::TextureDescriptor {
        label: None, size: wgpu::Extent3d { width:8, height:1, depth_or_array_layers:layers },
        mip_level_count:1, sample_count:1, dimension:wgpu::TextureDimension::D2,
        format, usage, view_formats:&[],
    });
    let depth = texture(wgpu::TextureFormat::Depth32Float, wgpu::TextureUsages::TEXTURE_BINDING, 1);
    let output = texture(wgpu::TextureFormat::Rgba16Float,
        wgpu::TextureUsages::STORAGE_BINDING|wgpu::TextureUsages::COPY_SRC,1);
    let grain = texture(wgpu::TextureFormat::Rgba8Unorm,wgpu::TextureUsages::TEXTURE_BINDING,64);
    let depth_view=depth.create_view(&Default::default());
    let output_view=output.create_view(&Default::default());
    let grain_view=grain.create_view(&Default::default());
    let buffer = |size,usage| device.create_buffer(&wgpu::BufferDescriptor {
        label:None,size,usage,mapped_at_creation:false,
    });
    let a=buffer(8*48,wgpu::BufferUsages::STORAGE);
    let b=buffer(8*48,wgpu::BufferUsages::STORAGE);
    let history=buffer(8*4,wgpu::BufferUsages::STORAGE|wgpu::BufferUsages::COPY_SRC);
    let readback=buffer(288,wgpu::BufferUsages::COPY_DST|wgpu::BufferUsages::MAP_READ);
    let bind=device.create_bind_group(&wgpu::BindGroupDescriptor {
        label:None,layout:&pipeline.get_bind_group_layout(0),entries:&[
            gpu::view(0,&depth_view),gpu::view(1,&output_view),
            wgpu::BindGroupEntry { binding:2,resource:a.as_entire_binding() },
            wgpu::BindGroupEntry { binding:3,resource:b.as_entire_binding() },
            gpu::view(4,&grain_view),
            wgpu::BindGroupEntry { binding:5,resource:history.as_entire_binding() },
        ],
    });
    let mut encoder=device.create_command_encoder(&Default::default());
    {
        let mut pass=encoder.begin_compute_pass(&Default::default());
        pass.set_pipeline(&pipeline);pass.set_bind_group(0,&bind,&[]);pass.dispatch_workgroups(1,1,1);
    }
    encoder.copy_texture_to_buffer(output.as_image_copy(),wgpu::TexelCopyBufferInfo {
        buffer:&readback,layout:wgpu::TexelCopyBufferLayout {
            offset:0,bytes_per_row:Some(256),rows_per_image:Some(1),
        },
    },output.size());
    encoder.copy_buffer_to_buffer(&history,0,&readback,256,32);
    queue.submit([encoder.finish()]);
    let (send,recv)=std::sync::mpsc::channel();
    readback.slice(..).map_async(wgpu::MapMode::Read,move |r|send.send(r).unwrap());
    device.poll(wgpu::PollType::wait_indefinitely()).unwrap();recv.recv().unwrap().unwrap();
    let mapped=readback.slice(..).get_mapped_range().unwrap();
    let values=mapped[..64].chunks_exact(8).map(|pixel| {
        std::array::from_fn(|i|half_to_float(u16::from_le_bytes([pixel[i*2],pixel[i*2+1]])))
    }).collect();
    let histories=mapped[256..288].chunks_exact(4)
        .map(|bytes|f32::from_le_bytes(bytes.try_into().unwrap())).collect();
    (values,histories)
}

#[test]
#[ignore = "Requires a graphics adapter; run for GI spatial-stage changes"]
fn practical_only_spatial_output_survives_and_invalid_surface_clears_history() {
    let (pixels,history)=fixture();
    assert_eq!(pixels[0],[2.0,4.0,8.0,2.0],"sun-off practical bounce was discarded");
    assert_eq!(pixels[1],[0.0;4],"no lighting should clear the output");
    assert_eq!(pixels[2],[2.0,4.0,8.0,2.0],"ordinary HDR must remain unchanged");
    assert_eq!(pixels[3],[0.0;4]);
    assert_eq!(history[3],0.0,"invalid geometry left a reusable stale reservoir");
}

#[test]
#[ignore = "Requires a graphics adapter; run for GI half-float output changes"]
fn gi_half_float_transport_is_finite_without_clipping_representable_hdr() {
    let (pixels,_)=fixture();
    assert_eq!(pixels[2],[2.0,4.0,8.0,2.0]);
    assert_eq!(pixels[4],[65504.0,65504.0,65504.0,2.0],"valid bright transport overflowed half-float");
    for (i,pixel) in pixels.iter().enumerate() {
        assert!(pixel.iter().all(|v|v.is_finite() && *v>=0.0),"case {i}: {pixel:?}");
    }
    assert_eq!(pixels[7],[2.0,4.0,8.0,65504.0],"depth overflow can poison bilateral weights");
}
