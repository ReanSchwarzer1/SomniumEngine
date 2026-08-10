//! The rendering context, encapsulating `wgpu` state.
//!
//! [`RenderContext`] holds the fundamental GPU resources required to
//! interact with the hardware: the `Instance`, `Adapter`, `Device`,
//! `Queue`, and `Surface`.
//!
//! ## Reference Architecture
//!
//! The initialization of these resources is abstracted here so that
//! `somnium_core::Engine` can initialize them synchronously during
//! the `resumed` lifecycle callback. This is similar to how Unreal
//! Engine 5's `FEngineLoop` initializes the RHI (Render Hardware
//! Interface) during its `Init` phase.

use std::sync::Arc;
use tracing::{info, warn};
use winit::window::Window;

/// Encapsulates the core `wgpu` state for the application.
pub struct RenderContext {
    /// The `wgpu` instance.
    pub instance: wgpu::Instance,
    /// The physical GPU adapter.
    pub adapter: wgpu::Adapter,
    /// The logical device.
    pub device: wgpu::Device,
    /// The command queue.
    pub queue: wgpu::Queue,
    /// The window surface.
    pub surface: wgpu::Surface<'static>,
    /// The surface configuration (size, format, present mode).
    pub config: wgpu::SurfaceConfiguration,
    /// Features actually granted by the device.
    ///
    /// Phase 15 checks this to decide whether the GPU-driven indirect path is
    /// available; anything optional must degrade gracefully when it isn't.
    pub features: wgpu::Features,
}

/// Optional features the GPU-driven renderer (Phase 15) needs.
///
/// `multi_draw_indirect` itself is core in wgpu 29 (it only needs the
/// `INDIRECT_EXECUTION` downlevel flag, and wgpu emulates it where a backend
/// lacks it). What *is* gated is `INDIRECT_FIRST_INSTANCE`: a non-zero
/// `first_instance` in the draw args, which is how each indirect draw finds its
/// slot in the instance buffer. Without it every draw would read instance 0, so
/// the renderer falls back to the per-draw CPU loop instead.
pub const GPU_DRIVEN_FEATURES: wgpu::Features = wgpu::Features::INDIRECT_FIRST_INSTANCE;

/// Feature needed to build and trace acceleration structures (Phase 24J).
///
/// In wgpu 29 `EXPERIMENTAL_RAY_QUERY` covers both building acceleration
/// structures and querying them; there is no separate flag for the former.
///
/// The gap to hardware ray tracing is genuinely just ray query: the binding
/// arrays and non-uniform indexing that Bevy's Solari also requires are already
/// mandatory here for the bindless resource pool. Still experimental in wgpu
/// and effectively Vulkan-only, so it is detected rather than required — a GPU
/// without it must still start.
pub const RAY_TRACING_FEATURES: wgpu::Features = wgpu::Features::EXPERIMENTAL_RAY_QUERY;

/// Phase 29: GPU timestamps for the profiler.
///
/// `TIMESTAMP_QUERY` alone only permits timestamps written by a pass
/// descriptor's `timestamp_writes`, which would mean editing every pass in the
/// engine to hand it query indices. `TIMESTAMP_QUERY_INSIDE_ENCODERS` allows
/// them on the encoder between passes, so the profiler brackets a pass from
/// outside and the passes themselves stay unaware they are being measured.
pub const PROFILER_FEATURES: wgpu::Features = wgpu::Features::TIMESTAMP_QUERY
    .union(wgpu::Features::TIMESTAMP_QUERY_INSIDE_ENCODERS);

impl RenderContext {
    /// Create a new `RenderContext` asynchronously.
    ///
    /// This requests a high-performance adapter and requires specific
    /// modern rendering features (like bindless support).
    ///
    /// # Panics
    ///
    /// Panics if no suitable adapter is found or if the required features
    /// are not supported by the hardware.
    pub async fn new(window: Arc<Window>) -> Self {
        let size = window.inner_size();
        let width = size.width.max(1);
        let height = size.height.max(1);

        // Create the wgpu instance with Vulkan/DX12/Metal backends.
        let instance = wgpu::Instance::default();

        // Create the surface from the window.
        // SAFETY: The surface must outlive the window. We use an Arc<Window>
        // and create the surface with a 'static lifetime by cloning the Arc.
        let surface = instance
            .create_surface(window)
            .expect("Failed to create wgpu surface");

        // Request a high-performance adapter.
        let adapter = instance
            .request_adapter(&wgpu::RequestAdapterOptions {
                power_preference: wgpu::PowerPreference::HighPerformance,
                compatible_surface: Some(&surface),
                force_fallback_adapter: false,
            })
            .await
            .expect("No suitable GPU adapter found");

        let info = adapter.get_info();
        info!(
            backend = ?info.backend,
            device = %info.name,
            device_type = ?info.device_type,
            "Selected GPU adapter"
        );

        // We require specific features for modern rendering (Bindless).
        let required_features = wgpu::Features::TEXTURE_BINDING_ARRAY
            | wgpu::Features::SAMPLED_TEXTURE_AND_STORAGE_BUFFER_ARRAY_NON_UNIFORM_INDEXING
            | wgpu::Features::PARTIALLY_BOUND_BINDING_ARRAY
            | wgpu::Features::PRIMITIVE_INDEX;

        let available_features = adapter.features();
        if !available_features.contains(required_features) {
            warn!("GPU does not support full bindless rendering. Attempting to request anyway, which may fail.");
        }

        // Phase 15: ask for the GPU-driven draw features only if the adapter has
        // them, so a GPU without them still starts (just on the CPU draw path).
        let gpu_driven = available_features.contains(GPU_DRIVEN_FEATURES);
        if gpu_driven {
            info!("GPU-driven rendering available (multi-draw indirect)");
        } else {
            info!("GPU-driven rendering unavailable — using the per-draw CPU path");
        }
        let required_features = if gpu_driven {
            required_features | GPU_DRIVEN_FEATURES
        } else {
            required_features
        };

        // Phase 24J: same pattern — detect, do not demand.
        let ray_tracing = available_features.contains(RAY_TRACING_FEATURES);
        if ray_tracing {
            info!("Hardware ray tracing available (acceleration structures + ray query)");
        } else {
            info!("Hardware ray tracing unavailable — the GI path will need the software fallback");
        }
        let required_features = if ray_tracing {
            required_features | RAY_TRACING_FEATURES
        } else {
            required_features
        };

        // Phase 29: detect, do not demand — same as the two above. A GPU
        // without timestamps still runs; it just has no profiler.
        let timestamps = available_features.contains(PROFILER_FEATURES);
        if timestamps {
            info!("GPU timestamp queries available (profiler)");
        } else {
            info!("GPU timestamp queries unavailable — the profiler will show counters only");
        }
        let required_features = if timestamps {
            required_features | PROFILER_FEATURES
        } else {
            required_features
        };

        let mut limits = wgpu::Limits::default();
        limits.max_binding_array_elements_per_shader_stage = 1024;
        limits.max_storage_buffers_per_shader_stage = 16;
        // Phase 24AC: SPD writes six mip levels from one dispatch, and wgpu's
        // default ceiling is four storage textures per stage. Asked for from
        // the adapter rather than demanded — a device that cannot manage six
        // falls back to the per-mip pyramid, which is why `HiZPass` checks the
        // granted limit rather than assuming this succeeded.
        limits.max_storage_textures_per_shader_stage = adapter
            .limits()
            .max_storage_textures_per_shader_stage
            .min(8)
            .max(limits.max_storage_textures_per_shader_stage);
        // Phase 17E: the geometry pool is a storage buffer, and wgpu's default
        // ceiling of 128 MB is small for a scene holding a photoscanned model.
        // Ask for whatever this adapter actually supports and let the pool size
        // itself to the result — requesting more than the adapter allows fails
        // device creation outright.
        let adapter_limits = adapter.limits();
        limits.max_storage_buffer_binding_size = adapter_limits.max_storage_buffer_binding_size;
        limits.max_buffer_size = adapter_limits.max_buffer_size;

        // Phase 24J: acceleration-structure limits default to zero, so a TLAS
        // of any size is rejected until they are asked for explicitly. Only
        // requested when ray tracing was detected — a device that cannot ray
        // trace reports zero for these, and asking for more than the adapter
        // allows fails device creation outright.
        if ray_tracing {
            limits.max_blas_primitive_count = adapter_limits.max_blas_primitive_count;
            limits.max_blas_geometry_count = adapter_limits.max_blas_geometry_count;
            limits.max_tlas_instance_count = adapter_limits.max_tlas_instance_count;
            // Binding one is also a limit, and also defaults to zero.
            limits.max_acceleration_structures_per_shader_stage =
                adapter_limits.max_acceleration_structures_per_shader_stage;
        }

        // Request the device and queue.
        let (device, queue) = adapter
            .request_device(
                &wgpu::DeviceDescriptor {
                    label: Some("somnium_device"),
                    required_features,
                    required_limits: limits,
                    // Ray query is gated behind an explicit acknowledgement
                    // token, not just a feature bit. wgpu is asking the caller
                    // to accept that its experimental APIs may contain
                    // soundness bugs reachable from otherwise-safe code — the
                    // token is `unsafe` precisely so that acceptance is
                    // deliberate rather than incidental.
                    //
                    // Taken only when ray tracing was actually detected, so a
                    // machine that cannot ray trace never opts into the risk.
                    //
                    // SAFETY: no safety obligation can be discharged here; this
                    // is an acknowledgement, and it is scoped as narrowly as
                    // the API allows.
                    experimental_features: if ray_tracing {
                        unsafe { wgpu::ExperimentalFeatures::enabled() }
                    } else {
                        wgpu::ExperimentalFeatures::disabled()
                    },
                    ..Default::default()
                },
            )
            .await
            .expect("Failed to request wgpu device with required features");

        let surface_caps = surface.get_capabilities(&adapter);
        
        // Prefer sRGB format for the swapchain.
        let surface_format = surface_caps
            .formats
            .iter()
            .copied()
            .find(|f| f.is_srgb())
            .unwrap_or(surface_caps.formats[0]);

        let config = wgpu::SurfaceConfiguration {
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            format: surface_format,
            width,
            height,
            present_mode: wgpu::PresentMode::AutoVsync,
            alpha_mode: surface_caps.alpha_modes[0],
            view_formats: vec![],
            desired_maximum_frame_latency: 2,
        };

        surface.configure(&device, &config);

        let features = device.features();

        Self {
            instance,
            adapter,
            device,
            queue,
            surface,
            config,
            features,
        }
    }

    /// Whether the GPU-driven indirect draw path (Phase 15) can be used.
    pub fn supports_gpu_driven(&self) -> bool {
        self.features.contains(GPU_DRIVEN_FEATURES)
    }

    /// Resize the surface.
    pub fn resize(&mut self, width: u32, height: u32) {
        if width > 0 && height > 0 {
            self.config.width = width;
            self.config.height = height;
            self.surface.configure(&self.device, &self.config);
        }
    }
}
