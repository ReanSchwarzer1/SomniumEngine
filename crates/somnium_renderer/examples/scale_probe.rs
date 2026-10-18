//! How the renderer holds up as one scene grows to a whole region.
//!
//! Opens a hidden window and the real renderer, spreads N static instances (houses, props, tree trunks) over a
//! square region at game scale, and draws frames from a high three-quarter camera, the way an isometric game sees a
//! town. For each N it prints the wall time of submission and of the frame, and the renderer's own CPU and GPU
//! zones, so whatever grows faster than the rest shows itself.
//!
//! ```text
//! cargo run --release -p somnium_renderer --example scale_probe -- 5000 20000 50000 100000 180000
//! ```
//!
//! One window: check `tasklist` and hold the shared native-game lock first (Rimeborn RULES 12).
use somnium_renderer::SomniumRenderer as Renderer;
use somnium_renderer::command::{DrawCommand, SortKey};
use somnium_renderer::context::RenderContext;
use somnium_renderer::material::pool::GpuMaterial;
use std::sync::Arc;
use std::time::Instant;
use winit::application::ApplicationHandler;
use winit::event::WindowEvent;
use winit::event_loop::{ActiveEventLoop, EventLoop};
use winit::window::{Window, WindowAttributes, WindowId};

const WARM_FRAMES: usize = 20;
const TIMED_FRAMES: usize = 30;

struct Probe {
    counts: Vec<usize>,
    done: bool,
}

/// Wait for a future on this thread (the renderer's context is built async).
fn block_on<T>(future: impl std::future::Future<Output = T>) -> T {
    use std::task::{Context, Poll, Wake, Waker};
    struct ThreadWake(std::thread::Thread);
    impl Wake for ThreadWake {
        fn wake(self: Arc<Self>) {
            self.0.unpark();
        }
    }
    let waker = Waker::from(Arc::new(ThreadWake(std::thread::current())));
    let mut context = Context::from_waker(&waker);
    let mut future = Box::pin(future);
    loop {
        match future.as_mut().poll(&mut context) {
            Poll::Ready(value) => return value,
            Poll::Pending => std::thread::park(),
        }
    }
}

fn material(renderer: &mut Renderer, queue: &wgpu::Queue, rgb: [f32; 3]) -> u32 {
    renderer.materials_pool.add_material(
        queue,
        GpuMaterial {
            base_color: [rgb[0], rgb[1], rgb[2], 1.0],
            roughness: 0.8,
            metallic: 0.0,
            albedo_map: -1,
            normal_map: -1,
            metallic_roughness_map: -1,
            alpha_cutoff: 0.0,
            flags: 0,
            occlusion_map: -1,
            transmission: 0.0,
            emissive: [0.0; 3],
            emissive_map: -1,
            terrain_index: -1,
            porosity: 0.5,
            normal_scale: 1.0,
            height_map: -1,
            height_depth: 0.0,
            weathering: 0.0,
            detile: 0.0,
            wind: [0.0; 2],
            subsurface: 0.0,
            reserved: 0.0,
        },
    )
}

/// A cheap, repeatable scatter: the n-th point of a jittered grid.
fn hash(i: u32, salt: u32) -> f32 {
    let mut x = i.wrapping_mul(0x9E37_79B9) ^ salt.wrapping_mul(0x85EB_CA6B);
    x ^= x >> 15;
    x = x.wrapping_mul(0x2C1B_3C6D);
    x ^= x >> 12;
    (x & 0xFFFF) as f32 / 65535.0
}

impl Probe {
    fn run(&self, window: Arc<Window>) {
        let mut ctx = block_on(RenderContext::new(Arc::clone(&window)));
        ctx.set_vsync(false);
        let mut ui = somnium_ui::UiManager::new(
            &ctx.device,
            ctx.config.format,
            1,
            &ctx.queue,
            Arc::clone(&window),
        );
        let mut renderer = Renderer::new(&ctx);
        renderer.profiler.set_enabled(true);
        let wall = material(&mut renderer, &ctx.queue, [0.78, 0.72, 0.62]);
        let wood = material(&mut renderer, &ctx.queue, [0.45, 0.33, 0.22]);
        let stone = material(&mut renderer, &ctx.queue, [0.55, 0.55, 0.58]);
        let (cube_v, cube_i) = somnium_asset::generate_cube(1.0);
        let (sphere_v, sphere_i) = somnium_asset::generate_sphere(0.5, 16, 16);
        let (cyl_v, cyl_i) = somnium_asset::generate_cylinder(0.5, 1.0, 16);
        let meshes = [
            (
                renderer
                    .geometry
                    .upload_mesh(&ctx.queue, &cube_v, &cube_i, wall),
                wall,
            ),
            (
                renderer
                    .geometry
                    .upload_mesh(&ctx.queue, &sphere_v, &sphere_i, stone),
                stone,
            ),
            (
                renderer
                    .geometry
                    .upload_mesh(&ctx.queue, &cyl_v, &cyl_i, wood),
                wood,
            ),
        ];
        println!("adapter: {}", ctx.adapter.get_info().name);
        println!(
            "gpu-driven: {}, counted draws: {}",
            ctx.supports_gpu_driven(),
            ctx.supports_counted_draws()
        );
        for &n in &self.counts {
            // Twelve metres apart on average, as a town's houses, props and trees are: 50,000 cover about 2.7 km.
            let side = ((n as f32).sqrt() * 12.0).max(12.0);
            let per_row = (n as f32).sqrt().ceil() as u32;
            let draws: Vec<DrawCommand> = (0..n as u32)
                .map(|i| {
                    let (gx, gz) = ((i % per_row) as f32, (i / per_row) as f32);
                    let x = (gx + hash(i, 1)) * 12.0 - side * 0.5;
                    let z = (gz + hash(i, 2)) * 12.0 - side * 0.5;
                    let kind = (i % 3) as usize;
                    let scale = match kind {
                        0 => glam::Vec3::new(
                            6.0 + 4.0 * hash(i, 3),
                            4.0 + 3.0 * hash(i, 4),
                            6.0 + 4.0 * hash(i, 5),
                        ),
                        1 => glam::Vec3::splat(0.8 + 0.6 * hash(i, 3)),
                        _ => glam::Vec3::new(0.5, 6.0 + 4.0 * hash(i, 4), 0.5),
                    };
                    let transform = glam::Mat4::from_scale_rotation_translation(
                        scale,
                        glam::Quat::from_rotation_y(hash(i, 6) * std::f32::consts::TAU),
                        glam::Vec3::new(x, scale.y * 0.5, z),
                    );
                    let (alloc, material_id) = &meshes[kind];
                    DrawCommand {
                        sort_key: SortKey::new(0, *material_id as u16, i),
                        vertex_offset: alloc.vertex_offset,
                        index_offset: alloc.index_offset,
                        index_count: alloc.index_count,
                        material_id: *material_id,
                        transform,
                        casts_shadow: true,
                        paint: 0,
                    }
                })
                .collect();
            // A high three-quarter view of the middle of the region, about 120 m across.
            // `SCALE_PROBE_MOVE=1` pans 2 m a frame, as a player walking does, which moves the shadow cascades.
            let pan = std::env::var("SCALE_PROBE_MOVE").as_deref() == Ok("1");
            let proj = glam::Mat4::perspective_rh(0.6, 16.0 / 9.0, 0.5, 4000.0);
            let mut submit_ms = 0.0;
            let mut frame_ms = 0.0;
            // Acquire, then encoder finish, queue submit, the work between, present.
            let mut outside = [0.0f64; 5];
            for frame in 0..WARM_FRAMES + TIMED_FRAMES {
                let start = Instant::now();
                let target = if pan { glam::Vec3::new(frame as f32 * 2.0, 0.0, 0.0) } else { glam::Vec3::ZERO };
                let eye = target + glam::Vec3::new(55.0, 70.0, 55.0);
                let view = glam::Mat4::look_at_rh(eye, target, glam::Vec3::Y);
                renderer.set_view(view, proj, eye);
                for draw in &draws {
                    renderer.submit(draw.clone());
                }
                let submitted = Instant::now();
                renderer.render(&ctx, &mut ui, &window);
                let end = Instant::now();
                if frame >= WARM_FRAMES {
                    submit_ms += (submitted - start).as_secs_f64() * 1000.0;
                    frame_ms += (end - submitted).as_secs_f64() * 1000.0;
                    outside[0] += f64::from(renderer.profiler.surface_acquire_ms);
                    for (sum, part) in outside[1..].iter_mut().zip(renderer.profiler.submit_parts_ms) {
                        *sum += f64::from(part);
                    }
                }
            }
            let frames = TIMED_FRAMES as f64;
            println!("\n=== {n} instances over {side:.0} m ===");
            println!(
                "submit {:.3} ms   render call {:.3} ms (wall, mean of {TIMED_FRAMES})",
                submit_ms / frames,
                frame_ms / frames
            );
            let [acquire, finish, queue_submit, between, present] = outside.map(|ms| ms / frames);
            println!(
                "outside the zones: acquire {acquire:.3}  finish {finish:.3}  submit {queue_submit:.3}  then {between:.3}  present {present:.3} ms"
            );
            for line in renderer.profiler.report() {
                println!("  {line}");
            }
        }
    }
}

impl ApplicationHandler for Probe {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.done {
            return;
        }
        let window = Arc::new(
            event_loop
                .create_window(
                    WindowAttributes::default()
                        .with_title("scale probe")
                        .with_visible(false)
                        .with_inner_size(winit::dpi::PhysicalSize::new(1920, 1080)),
                )
                .expect("a window"),
        );
        self.run(window);
        self.done = true;
        event_loop.exit();
    }

    fn window_event(&mut self, _event_loop: &ActiveEventLoop, _id: WindowId, _event: WindowEvent) {}
}

fn main() {
    let counts: Vec<usize> = std::env::args()
        .skip(1)
        .filter_map(|a| a.parse().ok())
        .collect();
    let mut probe = Probe {
        counts: if counts.is_empty() {
            vec![5_000, 20_000, 50_000]
        } else {
            counts
        },
        done: false,
    };
    EventLoop::new()
        .expect("an event loop")
        .run_app(&mut probe)
        .expect("the probe runs");
}
