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
/// `multi_draw_indirect` itself is core in wgpu 30 (it only needs the
/// `INDIRECT_EXECUTION` downlevel flag, and wgpu emulates it where a backend
/// lacks it). What *is* gated is `INDIRECT_FIRST_INSTANCE`: a non-zero
/// `first_instance` in the draw args, which is how each indirect draw finds its
/// slot in the instance buffer. Without it every draw would read instance 0, so
/// the renderer falls back to the per-draw CPU loop instead.
pub const GPU_DRIVEN_FEATURES: wgpu::Features = wgpu::Features::INDIRECT_FIRST_INSTANCE;

/// DOOM-G: GPU-authored draw counts for compacted indirect streams.
///
/// Optional even when the dense GPU-driven path is available. Without it the
/// cull shader keeps zeroing `instance_count` in place and the visibility pass
/// uses `multi_draw_indirect`, preserving the Phase 15 fallback exactly.
pub const DRAW_COUNT_FEATURES: wgpu::Features = wgpu::Features::MULTI_DRAW_INDIRECT_COUNT;

/// Feature needed to build and trace acceleration structures (Phase 24J).
///
/// In wgpu 30 `EXPERIMENTAL_RAY_QUERY` still covers both building acceleration
/// structures and querying them; there is no separate flag for the former.
/// wgpu 30 does add `ACCELERATION_STRUCTURE_BINDING_ARRAY` and
/// `EXPERIMENTAL_RAY_HIT_VERTEX_RETURN` beside it — neither is required here,
/// and both are probed by `crate::capability` for MORROWIND-U's benefit.
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
pub const PROFILER_FEATURES: wgpu::Features =
    wgpu::Features::TIMESTAMP_QUERY.union(wgpu::Features::TIMESTAMP_QUERY_INSIDE_ENCODERS);

/// Phase DOOM-A: pipeline statistics, for the "why" beside the "how long".
///
/// A pass time says how long something took and never why. The counters that
/// answer it — how many fragments the rasterizer actually issued, how many
/// compute invocations a dispatch really ran — are only available through this
/// feature, and unlike `TIMESTAMP_QUERY_INSIDE_ENCODERS` there is no
/// outside-the-pass form: `begin_pipeline_statistics_query` lives on the render
/// or compute pass itself. So passes opt in one at a time rather than the
/// profiler bracketing them all, and only the passes worth the wiring have it.
pub const PIPELINE_STATS_FEATURES: wgpu::Features = wgpu::Features::PIPELINE_STATISTICS_QUERY;

/// Phase XV-E: optional BCn texture compression (BC7 for terrain packs).
///
/// Detect, do not demand — a GPU without BC still starts on the RGBA8 path.
/// Never request this bit if the adapter lacks it.
pub const BC_COMPRESSION_FEATURES: wgpu::Features = wgpu::Features::TEXTURE_COMPRESSION_BC;

/// Features FSR 3 needs on the device (detect, do not demand).
///
/// wgpu-ffx's tests request *every* adapter feature. We ask for the set it
/// actually uses so a later `create_texture` / `create_shader_module` / encode
/// does not panic on a missing bit:
/// - `TEXTURE_ADAPTER_SPECIFIC_FORMAT_FEATURES` — `rg16float` / `r16float` UAVs
/// - `PASSTHROUGH_SHADERS` — AMD SPIR-V uses GLSL `coherent` on images; naga
///   only allows `@coherent` on storage buffers, so wgpu-ffx passthroughs SPIR-V
/// - `TEXTURE_FORMAT_16BIT_NORM` — Lanczos2 LUT is `r16snorm`
/// - `FLOAT32_FILTERABLE` — dilated depth is `r32float` sampled as filterable
/// - `CLEAR_TEXTURE` — dispatch clears accumulation / SPD mips with `clear_texture`
pub const FSR_FEATURES: wgpu::Features = wgpu::Features::TEXTURE_ADAPTER_SPECIFIC_FORMAT_FEATURES
    .union(wgpu::Features::PASSTHROUGH_SHADERS)
    .union(wgpu::Features::TEXTURE_FORMAT_16BIT_NORM)
    .union(wgpu::Features::FLOAT32_FILTERABLE)
    .union(wgpu::Features::CLEAR_TEXTURE);

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
                // wgpu 30. Limit bucketing rounds the adapter's reported limits
                // to pre-defined buckets so untrusted content cannot fingerprint
                // the GPU. Somnium is a native editor, not a browser, and it
                // wants the real numbers: the terrain pack sizing and the
                // bindless pool both read actual limits.
                apply_limit_buckets: false,
            })
            .await
            .expect("No suitable GPU adapter found");

        let info = adapter.get_info();
        info!(
            backend = ?info.backend,
            device = %info.name,
            vendor = info.vendor,
            device_id = info.device,
            device_type = ?info.device_type,
            driver = %info.driver,
            driver_info = %info.driver_info,
            "Selected GPU adapter"
        );

        // MORROWIND-A2: probe, do not trust. Logs one summary line, and writes
        // the full table when SOMNIUM_CAPABILITY_REPORT names a path.
        crate::capability::report(&adapter);

        // We require specific features for modern rendering (Bindless).
        let required_features = wgpu::Features::TEXTURE_BINDING_ARRAY
            | wgpu::Features::SAMPLED_TEXTURE_AND_STORAGE_BUFFER_ARRAY_NON_UNIFORM_INDEXING
            | wgpu::Features::PARTIALLY_BOUND_BINDING_ARRAY
            | wgpu::Features::PRIMITIVE_INDEX;

        let available_features = adapter.features();
        if !available_features.contains(required_features) {
            warn!(
                "GPU does not support full bindless rendering. Attempting to request anyway, which may fail."
            );
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

        // DOOM-G: counted submission is a second optional tier. The compacted
        // stream is only a draw consumer; dense args remain authoritative for
        // two-phase culling and diagnostics.
        let counted_draws = gpu_driven && available_features.contains(DRAW_COUNT_FEATURES);
        if counted_draws {
            info!("GPU-counted indirect submission available");
        } else if gpu_driven {
            info!("GPU-counted indirect submission unavailable — keeping dense zero-count args");
        }
        let required_features = if counted_draws {
            required_features | DRAW_COUNT_FEATURES
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

        // Phase DOOM-A: same pattern again. No adapter is required to have it,
        // and the profiler simply omits the counter rows when it does not.
        let pipeline_stats = available_features.contains(PIPELINE_STATS_FEATURES);
        if pipeline_stats {
            info!("Pipeline statistics available (fragment / compute invocation counters)");
        } else {
            info!("Pipeline statistics unavailable — profiler reports timings only");
        }
        let required_features = if pipeline_stats {
            required_features | PIPELINE_STATS_FEATURES
        } else {
            required_features
        };

        // Phase XV-E: same pattern — detect, do not demand. RGBA8 packs remain
        // the fallback; BC7 is never kept resident alongside them.
        let bc = available_features.contains(BC_COMPRESSION_FEATURES);
        if bc {
            info!("BC texture compression available (terrain BC7 packs eligible)");
        } else {
            info!("BC texture compression unavailable — terrain stays on RGBA8");
        }
        let required_features = if bc {
            required_features | BC_COMPRESSION_FEATURES
        } else {
            required_features
        };

        let fsr = available_features.contains(FSR_FEATURES);
        if fsr {
            info!("FSR 3 available (adapter storage formats + shader passthrough)");
        } else {
            info!("FSR 3 unavailable — missing adapter storage formats or shader passthrough");
        }
        let required_features = if fsr {
            required_features | FSR_FEATURES
        } else {
            required_features
        };

        let mut limits = wgpu::Limits::default();
        limits.max_binding_array_elements_per_shader_stage = 1024;
        limits.max_storage_buffers_per_shader_stage = 16;
        // Phase 24AC / FSR: SPD writes six mip UAVs from one dispatch, and FSR's
        // luma pyramid binds those plus two more. wgpu's default ceiling is four
        // storage textures per stage. Asked for from the adapter rather than
        // demanded — a device that cannot manage six falls back to the per-mip
        // pyramid, which is why `HiZPass` checks the granted limit rather than
        // assuming this succeeded.
        limits.max_storage_textures_per_shader_stage = adapter
            .limits()
            .max_storage_textures_per_shader_stage
            .min(16)
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
            .request_device(&wgpu::DeviceDescriptor {
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
            })
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

        let surface_usage = if surface_caps.usages.contains(wgpu::TextureUsages::COPY_SRC) {
            wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC
        } else {
            wgpu::TextureUsages::RENDER_ATTACHMENT
        };
        let config = wgpu::SurfaceConfiguration {
            usage: surface_usage,
            format: surface_format,
            // wgpu 30 made the presentation colour space explicit. `Auto` is
            // supported for every format the surface reports and reproduces
            // wgpu 29's behaviour exactly, so the bump does not change what
            // reaches the display. Choosing an HDR space here is a rendering
            // decision with its own evidence, and A2 adds no feature.
            color_space: wgpu::SurfaceColorSpace::Auto,
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

    /// Whether DOOM-G may issue `multi_draw_indirect_count`.
    pub fn supports_counted_draws(&self) -> bool {
        self.features.contains(DRAW_COUNT_FEATURES)
    }

    /// Whether BC7 terrain packs may be uploaded (Phase XV-E).
    pub fn supports_bc_compression(&self) -> bool {
        self.features.contains(BC_COMPRESSION_FEATURES)
    }

    /// Whether FSR 3 may create `Rg16Float` storage images and load AMD SPIR-V.
    pub fn supports_fsr(&self) -> bool {
        self.features.contains(FSR_FEATURES)
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
