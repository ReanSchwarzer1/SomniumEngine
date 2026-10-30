//! Faces of two things in one plane: the visibility pass keeps the same one every frame under FSR's jitter once their
//! depth ties are apart, even one step apart (29 Oct 2026; visibility.wgsl `depth_ties`, renderer.rs `depth_tie`).
//!
//! The developer saw "a lot of z fighting with tables stacked" with FSR on: tables standing half inside each other, their
//! tops in one plane. Two floor quads stand in for them here, in one plane but placed by different transforms and cut
//! along different diagonals, so their depths round differently; the camera is the game's (74 degrees, near 0.035 m).
//! Over 16 jittered frames each pixel inside the overlap should show one quad throughout, with ties one step apart:
//! near the world's origin, at the Town's canal backdrop (where world positions carry less precision), there out to
//! 11 m (just short of where the ties fade), seen edge-on from 0.3 m off the plane, and seen face-on from 8 m; then
//! at the game's own pixels (a 96-row crop of its 965 rows at 74 degrees, so a step is what the game draws, not held
//! at its clamps): standing, a plane 8 m off at 45 degrees, and edge-on.
//! `DEPTH_TIE_SLOPE_SCALE=0.5` halves the slope's part of each step, the margin it is chosen with (it passes; 0.25
//! does not, edge-on first); depth_tie.wgsl's `DEPTH_TIE_STEP_TURNED` at half (1.2e-7) passes too. Before positions
//! were taken from the eye, at 45 degrees and 8 m one copy stood nearer by rounding alone, more than a step could move.
use somnium_renderer::{material::pool::GpuMaterial, pass::visibility::VisibilityBufferPass, shaders::Shaders};
use wgpu::util::DeviceExt;
#[path = "support/gpu.rs"]
mod gpu;

const SIZE: u32 = 96;
const FRAMES: usize = 16;

fn halton(mut i: u32, base: u32) -> f32 {
    let (mut f, mut r) = (1.0, 0.0);
    while i > 0 {
        f /= base as f32;
        r += f * (i % base) as f32;
        i /= base;
    }
    r
}

fn storage(device: &wgpu::Device, contents: &[u8]) -> wgpu::Buffer {
    device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: None,
        contents,
        usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
    })
}

fn entry(binding: u32, ty: wgpu::BindingType, count: Option<std::num::NonZeroU32>) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::VERTEX | wgpu::ShaderStages::FRAGMENT,
        ty,
        count,
    }
}

#[test]
#[ignore = "Requires a bindless-capable graphics adapter; run for visibility or depth changes"]
fn coplanar_faces_keep_one_winner_under_jitter_once_their_ties_are_apart() {
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
    let adapter = gpu::block_on(instance.request_adapter(&Default::default())).unwrap();
    let (device, queue) = gpu::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
        required_features: wgpu::Features::TEXTURE_BINDING_ARRAY
            | wgpu::Features::SAMPLED_TEXTURE_AND_STORAGE_BUFFER_ARRAY_NON_UNIFORM_INDEXING,
        required_limits: adapter.limits(),
        ..Default::default()
    }))
    .unwrap();

    // Quad A as it stands; quad B turned 0.7 rad about the vertical and moved, its corners chosen so it lies in the
    // same plane (y = -1) and overlaps A. Each vertex: position, normal, uv. `place` puts the pair in the world.
    let model_b = glam::Mat4::from_translation(glam::vec3(0.3, 0.0, -4.6)) * glam::Mat4::from_rotation_y(0.7);
    let corner = |p: glam::Vec3| [p.x, p.y, p.z, 0.0, 1.0, 0.0, 0.0, 0.0];
    let mut vertices: Vec<[f32; 8]> =
        [(-2.0, -2.0), (2.0, -2.0), (2.0, -8.0), (-2.0, -8.0)].iter().map(|&(x, z)| corner(glam::vec3(x, -1.0, z))).collect();
    vertices.extend([(-2.2, 2.2), (2.2, 2.2), (2.2, -2.2), (-2.2, -2.2)].iter().map(|&(x, z)| corner(glam::vec3(x, -1.0, z))));
    let indices: [u32; 12] = [0, 1, 2, 0, 2, 3, 0, 1, 3, 1, 2, 3];
    let instance_bytes = |model: glam::Mat4, vertex_offset: u32, index_offset: u32| {
        let mut b = bytemuck::bytes_of(&model.to_cols_array()).to_vec();
        b.extend(bytemuck::bytes_of(&[0u32, vertex_offset, index_offset, 0u32]));
        b
    };
    let instances_at = |place: glam::Mat4| {
        let mut b = instance_bytes(place, 0, 0);
        b.extend(instance_bytes(place * model_b, 4, 6));
        b
    };
    let material = GpuMaterial { albedo_map: -1, ..bytemuck::Zeroable::zeroed() };

    let vertex_buffer = storage(&device, bytemuck::cast_slice(&vertices));
    let index_buffer = storage(&device, bytemuck::cast_slice(&indices));
    let instance_buffer = storage(&device, &instances_at(glam::Mat4::IDENTITY));
    let view_buffer = storage(&device, &[0u8; 256]);
    let material_buffer = storage(&device, bytemuck::bytes_of(&material));
    let texture = gpu::texture(&device, 1, wgpu::TextureFormat::Rgba8Unorm, wgpu::TextureUsages::TEXTURE_BINDING);
    let texture_view = texture.create_view(&Default::default());
    let buffer = wgpu::BindingType::Buffer {
        ty: wgpu::BufferBindingType::Storage { read_only: true },
        has_dynamic_offset: false,
        min_binding_size: None,
    };
    let texture_ty = wgpu::BindingType::Texture {
        sample_type: wgpu::TextureSampleType::Float { filterable: true },
        view_dimension: wgpu::TextureViewDimension::D2,
        multisampled: false,
    };
    let global_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: None,
        entries: &[
            entry(0, buffer, None),
            entry(1, buffer, None),
            entry(2, buffer, None),
            entry(3, buffer, None),
            entry(4, texture_ty, std::num::NonZeroU32::new(1)),
            entry(5, buffer, None),
        ],
    });
    let global = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: None,
        layout: &global_layout,
        entries: &[
            wgpu::BindGroupEntry { binding: 0, resource: vertex_buffer.as_entire_binding() },
            wgpu::BindGroupEntry { binding: 1, resource: index_buffer.as_entire_binding() },
            wgpu::BindGroupEntry { binding: 2, resource: instance_buffer.as_entire_binding() },
            wgpu::BindGroupEntry { binding: 3, resource: view_buffer.as_entire_binding() },
            wgpu::BindGroupEntry { binding: 4, resource: wgpu::BindingResource::TextureViewArray(&[&texture_view]) },
            wgpu::BindGroupEntry { binding: 5, resource: material_buffer.as_entire_binding() },
        ],
    });
    let mut vis = VisibilityBufferPass::new(&device, &Shaders::new(), SIZE, SIZE, &global_layout);

    // The game's camera (player.rs: near 0.035 m, far 350 m), placed with the pair: where it stands, what it looks at,
    // its up and its height in degrees (74 in the game; narrower edge-on, so the plane still fills enough pixels).
    struct Camera {
        eye: glam::Vec3,
        look: glam::Vec3,
        up: glam::Vec3,
        fovy: f32,
    }
    let standing = Camera { eye: glam::vec3(0.0, 0.6, 0.5), look: glam::vec3(0.2, -1.0, -5.0), up: glam::Vec3::Y, fovy: 74.0 };

    // Which instance (+1, 0 for none) each pixel shows in each of the jittered frames, drawing `draws` with `ties`.
    let mut film = |place: glam::Mat4, camera: &Camera, ties: [u32; 2], draws: &[u32]| -> Vec<Vec<u32>> {
        queue.write_buffer(&instance_buffer, 0, &instances_at(place));
        vis.set_depth_ties(&device, &queue, &ties);
        let proj = glam::Mat4::perspective_rh(camera.fovy.to_radians(), 1.0, 0.035, 350.0);
        let slope = somnium_renderer::pass::visibility::depth_tie_slope(&proj, SIZE)
            * std::env::var("DEPTH_TIE_SLOPE_SCALE").ok().and_then(|s| s.parse::<f32>().ok()).unwrap_or(1.0);
        let eye = place.transform_point3(camera.eye);
        let view = glam::Mat4::look_at_rh(eye, place.transform_point3(camera.look), camera.up);
        (0..FRAMES)
            .map(|frame| {
                // FSR's jitter (renderer.rs `set_view`): an NDC offset on the projection's z axis.
                let j = glam::vec2(halton(frame as u32 + 1, 2) - 0.5, halton(frame as u32 + 1, 3) - 0.5);
                let mut jittered = proj;
                jittered.z_axis.x += j.x * 2.0 / SIZE as f32;
                jittered.z_axis.y -= j.y * 2.0 / SIZE as f32;
                let view_proj = jittered * view;
                let mut bytes = Vec::with_capacity(256);
                bytes.extend(bytemuck::bytes_of(&view_proj.to_cols_array()));
                bytes.extend(bytemuck::bytes_of(&view_proj.inverse().to_cols_array()));
                bytes.extend(bytemuck::bytes_of(&view.to_cols_array()));
                bytes.extend(bytemuck::bytes_of(&[eye.x, eye.y, eye.z, 0.0, 0.0, 1.0, slope, 0.0]));
                bytes.extend([0u8; 32]);
                queue.write_buffer(&view_buffer, 0, &bytes);
                let mut encoder = device.create_command_encoder(&Default::default());
                {
                    let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                        label: None,
                        color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                            view: &vis.view,
                            resolve_target: None,
                            depth_slice: None,
                            ops: wgpu::Operations { load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT), store: wgpu::StoreOp::Store },
                        })],
                        depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                            view: &vis.depth_view,
                            depth_ops: Some(wgpu::Operations { load: wgpu::LoadOp::Clear(1.0), store: wgpu::StoreOp::Store }),
                            stencil_ops: None,
                        }),
                        ..Default::default()
                    });
                    pass.set_pipeline(&vis.pipeline_two_sided);
                    pass.set_bind_group(0, &global, &[]);
                    pass.set_bind_group(1, &vis.cutout_bind_group, &[]);
                    for &i in draws {
                        pass.draw(0..6, i..i + 1);
                    }
                }
                let stride = SIZE * 8;
                let readback = device.create_buffer(&wgpu::BufferDescriptor {
                    label: None,
                    size: u64::from(stride * SIZE),
                    usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
                    mapped_at_creation: false,
                });
                encoder.copy_texture_to_buffer(
                    vis.texture.as_image_copy(),
                    wgpu::TexelCopyBufferInfo {
                        buffer: &readback,
                        layout: wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(stride), rows_per_image: None },
                    },
                    wgpu::Extent3d { width: SIZE, height: SIZE, depth_or_array_layers: 1 },
                );
                queue.submit([encoder.finish()]);
                readback.slice(..).map_async(wgpu::MapMode::Read, |_| {});
                device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
                let texels: Vec<u32> = bytemuck::cast_slice(&readback.slice(..).get_mapped_range().unwrap()).to_vec();
                texels.chunks(2).map(|t| t[0]).collect()
            })
            .collect()
    };

    let town = glam::Mat4::from_translation(glam::vec3(52.0, 1.5, -26.0));
    let edge_on = Camera { eye: glam::vec3(0.0, -0.7, 0.5), look: glam::vec3(0.2, -1.0, -5.0), up: glam::Vec3::Y, fovy: 10.0 };
    let face_on = Camera { eye: glam::vec3(0.3, 7.0, -4.6), look: glam::vec3(0.3, -1.0, -4.6), up: glam::Vec3::NEG_Z, fovy: 74.0 };
    // The game's pixels: 96 rows of its 965 at 74 degrees.
    let crop = (2.0 * ((37f32).to_radians().tan() * 96.0 / 965.0).atan()).to_degrees();
    let mid = glam::vec3(0.3, -1.0, -4.6);
    let game_standing = Camera { eye: glam::vec3(0.0, 0.6, 0.5), look: mid, up: glam::Vec3::Y, fovy: crop };
    let game_at_45 = Camera { eye: mid + glam::vec3(0.0, 5.66, 5.66), look: mid, up: glam::Vec3::Y, fovy: crop };
    let game_edge_on = Camera { eye: glam::vec3(0.0, -0.7, 0.5), look: mid, up: glam::Vec3::Y, fovy: crop };
    let cases = [
        ("origin", glam::Mat4::IDENTITY, &standing),
        ("Town", town, &standing),
        ("Town, to 11 m", town * glam::Mat4::from_scale(glam::Vec3::splat(1.3)), &standing),
        ("Town, edge-on from 0.3 m", town, &edge_on),
        ("Town, face-on from 8 m", town, &face_on),
        ("Town, standing, the game's pixels", town, &game_standing),
        ("Town, 8 m off at 45 degrees, the game's pixels", town, &game_at_45),
        ("Town, edge-on from 0.3 m, the game's pixels", town, &game_edge_on),
    ];
    let mut failed = Vec::new();
    for (name, place, camera) in cases {
        // The pixels each quad covers in every frame on its own: the overlap's inside, away from edges the jitter crosses.
        let alone_a = film(place, camera, [0, 0], &[0]);
        let alone_b = film(place, camera, [0, 0], &[1]);
        let inside: Vec<usize> = (0..(SIZE * SIZE) as usize)
            .filter(|&p| alone_a.iter().all(|f| f[p] == 1) && alone_b.iter().all(|f| f[p] == 2))
            .collect();
        assert!(inside.len() > 500, "{name}: the quads overlap over {} pixels only", inside.len());

        let flips = |frames: &[Vec<u32>]| inside.iter().filter(|&&p| frames.iter().any(|f| f[p] != frames[0][p])).count();
        let kept = |frames: &[Vec<u32>], id: u32| inside.iter().filter(|&&p| frames.iter().all(|f| f[p] == id)).count();
        let tied = film(place, camera, [0, 0], &[0, 1]);
        // One step apart: the closest two draws' ties can be.
        let a_over = film(place, camera, [1, 0], &[0, 1]);
        let b_over = film(place, camera, [0, 1], &[0, 1]);
        eprintln!(
            "{name}: {} pixels inside the overlap: {} change hands over {FRAMES} jittered frames with equal ties; one \
             step apart, {} with A's higher ({} kept A), {} with B's ({} kept B)",
            inside.len(),
            flips(&tied),
            flips(&a_over),
            kept(&a_over, 1),
            flips(&b_over),
            kept(&b_over, 2)
        );
        // `DEPTH_TIE_DUMP=dir`: a strip of the first four frames, equal ties above and B's a step higher below (A red,
        // B blue).
        if name == "origin" {
            if let Some(dir) = std::env::var_os("DEPTH_TIE_DUMP") {
                let mut strip = image::RgbImage::new(SIZE * 4, SIZE * 2);
                for (row, frames) in [&tied, &b_over].into_iter().enumerate() {
                    for (column, frame) in frames.iter().take(4).enumerate() {
                        for (p, &id) in frame.iter().enumerate() {
                            let colour = match id { 1 => [200, 60, 50], 2 => [60, 90, 210], _ => [20, 20, 20] };
                            let (x, y) = (p as u32 % SIZE, p as u32 / SIZE);
                            strip.put_pixel(column as u32 * SIZE + x, row as u32 * SIZE + y, image::Rgb(colour));
                        }
                    }
                }
                strip.save(std::path::Path::new(&dir).join("depth_tie_jitter.png")).unwrap();
            }
            assert!(flips(&tied) > 0, "equal ties should show the fight this guards against");
        }
        if flips(&a_over) != 0 || flips(&b_over) != 0 || kept(&a_over, 1) != inside.len() || kept(&b_over, 2) != inside.len()
        {
            failed.push(name);
        }
    }
    assert!(failed.is_empty(), "one step apart, a face did not win every frame everywhere in: {failed:?}");
}
