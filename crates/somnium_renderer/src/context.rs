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

/// Backends this build will consider, from `SOMNIUM_BACKEND` (or wgpu's own
/// `WGPU_BACKEND`).
///
/// `wgpu::Instance::default()` reads no environment at all, which is why asking
/// this engine for DX12 previously appeared to do nothing — there was no way to
/// choose, so an NVIDIA machine always landed on Vulkan.
///
/// It exists to make [`ray_query_compiler_is_safe`] testable rather than
/// permanent: that guard disables ray queries on NVIDIA + **Vulkan** because
/// the driver's shader compiler exhausts system memory building one, and
/// without a selector there was no way to ask whether another backend fares
/// better.
///
/// **Measured, on an RTX 5080 Laptop / driver 32.0.16.1656: DX12 is not an
/// escape hatch.** wgpu reports 3 of 13 features there, `RAY_QUERY` among the
/// missing, so DX12 loses hardware ray tracing anyway — and startup then fails
/// outright in `create_shader_module_passthrough`, because this engine's
/// precompiled Slang modules carry SPIR-V and no DXIL. Vulkan remains the right
/// backend on this hardware; the guard stands, and the raster fallbacks it
/// selects have to be good enough on their own.
///
/// Kept regardless, because a renderer that cannot be asked to try another
/// backend cannot answer that question next time either.
///
/// `None` leaves wgpu's own default untouched, so an unset variable is exactly
/// the behaviour this engine had before the selector existed.
fn requested_backends() -> Option<wgpu::Backends> {
    let raw = std::env::var("SOMNIUM_BACKEND")
        .or_else(|_| std::env::var("WGPU_BACKEND"))
        .ok()?;
    match raw.trim().to_ascii_lowercase().as_str() {
        "" => None,
        "dx12" | "d3d12" | "directx12" => Some(wgpu::Backends::DX12),
        "vulkan" | "vk" => Some(wgpu::Backends::VULKAN),
        "metal" | "mtl" => Some(wgpu::Backends::METAL),
        "gl" | "opengl" | "gles" => Some(wgpu::Backends::GL),
        other => {
            warn!("SOMNIUM_BACKEND={other} is not a backend name; leaving the default");
            None
        }
    }
}

/// Explicit opt-in that overrides [`ray_query_compiler_is_safe`].
///
/// The guard below is a workaround for a driver bug, not a statement about what
/// this engine can support, and a workaround with no way to re-test it becomes
/// permanent by default. `SOMNIUM_FORCE_RAY_QUERY=1` requests the feature
/// anyway, so a change meant to fix the underlying compile can be measured
/// without editing the guard out and forgetting to put it back.
///
/// **This can take a machine down, and not by crashing.** The failure it steps
/// around is an allocation that never completes: the driver's compiler consumed
/// more than 47 GB and the editor never reached its first frame. Run it under a
/// memory cap the first time — `tools/probe_ray_query.ps1` launches once, kills
/// the process at a ceiling, and reports the peak.
fn ray_query_forced() -> bool {
    std::env::var("SOMNIUM_FORCE_RAY_QUERY").as_deref() == Ok("1")
}

/// Kill switch for hardware ray tracing, independent of any guard below.
///
/// The lesson of GeForce 616.56 is not that one driver was bad; it is that a
/// driver can make this engine *unstartable* and the only recovery was editing
/// the source. `SOMNIUM_NO_RAY_QUERY=1` is the recovery that does not need a
/// rebuild, and it beats [`ray_query_forced`] deliberately — the escape hatch
/// must win over the override, or it is not an escape hatch.
fn ray_query_disabled() -> bool {
    std::env::var("SOMNIUM_NO_RAY_QUERY").as_deref() == Ok("1")
}

/// NVIDIA marketing driver versions whose Vulkan shader compiler cannot build
/// this engine's ray-query pipelines, in hundredths (616.56 is `61656`).
///
/// GeForce **616.56** (2026-08-26) allocated past 47 GB compiling ReSTIR-GI's
/// `initial_and_temporal` and never finished, taking the machine with it.
/// Measured, not inferred: four separate attempts to shrink the shader moved
/// peak memory by 4% total, and rolling the driver back fixed it outright on
/// unchanged engine code.
const RAY_QUERY_BROKEN_NVIDIA_DRIVERS: &[u32] = &[61656];

/// NVIDIA's marketing driver version, from whichever string the backend filled.
///
/// Neither shape that reaches us is the number NVIDIA's release notes use, which
/// is why the log line was hard to connect to the public reports:
///
/// - Windows/DX12 puts the INF version in `driver`, e.g. `32.0.16.1656`. The
///   marketing version is the last five digits of the final two components:
///   `16` + `1656` -> `161656` -> `61656` -> 616.56.
/// - Vulkan puts a decoded version in `driver_info`, e.g. `616.56` or
///   `616.56.0`, already in marketing form.
///
/// Returned in hundredths so the comparison is integer and exact.
fn nvidia_driver_version(driver: &str, driver_info: &str) -> Option<u32> {
    for raw in [driver_info, driver] {
        let text = raw.trim();
        if text.is_empty() {
            continue;
        }
        let parts: Vec<&str> = text.split('.').collect();
        // Windows INF form, checked first: its leading `32.0` would otherwise
        // parse as a plausible-looking marketing version of 32.00.
        if parts.len() == 4 {
            let joined = format!("{}{}", parts[2], parts[3]);
            if joined.len() >= 5 && joined.bytes().all(|b| b.is_ascii_digit()) {
                if let Ok(value) = joined[joined.len() - 5..].parse::<u32>() {
                    return Some(value);
                }
            }
        }
        // Vulkan form. The two-digit minor is what distinguishes a real
        // marketing version from the head of some other dotted string.
        if parts.len() >= 2 && parts[1].len() == 2 {
            if let (Ok(major), Ok(minor)) = (parts[0].parse::<u32>(), parts[1].parse::<u32>()) {
                return Some(major * 100 + minor);
            }
        }
    }
    None
}

/// Whether startup may request wgpu's experimental ray-query feature.
///
/// **Version-specific, not vendor-wide.** The original guard refused ray queries
/// on every NVIDIA + Vulkan machine, which was the right emergency measure and
/// the wrong permanent one: it cost ReSTIR DI, ReSTIR GI and traced water
/// reflections on the most common hardware this engine runs on, and it promoted
/// two never-exercised raster paths into the critical one. Exactly one driver
/// release was ever shown to fail.
///
/// Unknown versions are **allowed**. Failing closed would restore the blanket
/// ban through the back door every time a string fails to parse, and
/// `SOMNIUM_NO_RAY_QUERY=1` is the recovery if a future release repeats this.
fn ray_query_compiler_is_safe(
    backend: wgpu::Backend,
    vendor: u32,
    driver: &str,
    driver_info: &str,
) -> bool {
    const NVIDIA_VENDOR_ID: u32 = 0x10de;
    if backend != wgpu::Backend::Vulkan || vendor != NVIDIA_VENDOR_ID {
        return true;
    }
    match nvidia_driver_version(driver, driver_info) {
        Some(version) => !RAY_QUERY_BROKEN_NVIDIA_DRIVERS.contains(&version),
        None => true,
    }
}

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

/// DOOM-K. `enable f16;` in WGSL, and half-precision arithmetic in a shader.
///
/// Detected, never demanded, and requesting it does not change a single shader
/// on its own: a pipeline has to be compiled with the narrower types before any
/// of it reaches the GPU. The flag exists so the experiment can *ask* whether
/// the device would allow it, which is a different question from whether it
/// helps.
pub const SHADER_F16_FEATURES: wgpu::Features = wgpu::Features::SHADER_F16;

/// DOOM-L. Subgroup ballot, broadcast and reduction intrinsics in WGSL.
///
/// Same contract: detected and requested when present, and no default path may
/// depend on it — a device without subgroups must take a scalar or
/// workgroup-shared path that produces identical results.
pub const SUBGROUP_FEATURES: wgpu::Features = wgpu::Features::SUBGROUP;

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

        // Create the wgpu instance. `SOMNIUM_BACKEND` narrows the set; see
        // `requested_backends` for why that is load-bearing on NVIDIA.
        let mut descriptor = wgpu::InstanceDescriptor::new_without_display_handle();
        if let Some(backends) = requested_backends() {
            info!(?backends, "Backend selection overridden by the environment");
            descriptor.backends = backends;
        }
        let instance = wgpu::Instance::new(descriptor);

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
        let ray_query_available = available_features.contains(RAY_TRACING_FEATURES);
        let forced = ray_query_forced();
        let compiler_is_safe =
            ray_query_compiler_is_safe(info.backend, info.vendor, &info.driver, &info.driver_info);
        let ray_tracing =
            ray_query_available && !ray_query_disabled() && (forced || compiler_is_safe);
        if ray_query_available && ray_query_disabled() {
            warn!("SOMNIUM_NO_RAY_QUERY=1: hardware ray tracing disabled by request");
        }
        if ray_tracing && forced && !compiler_is_safe {
            warn!(
                backend = ?info.backend,
                driver = %info.driver_info,
                "SOMNIUM_FORCE_RAY_QUERY=1: requesting ray queries on a driver known to                  exhaust system memory compiling them. If startup stalls here, it is not hung                  — it is allocating, and it will not stop."
            );
        }
        if ray_tracing {
            info!("Hardware ray tracing available (acceleration structures + ray query)");
        } else if ray_query_available {
            warn!(
                backend = ?info.backend,
                driver = %info.driver_info,
                "Hardware ray tracing disabled: this exact driver release exhausts system memory \
                 compiling ray-query pipelines. Roll back to the previous branch, or set \
                 SOMNIUM_FORCE_RAY_QUERY=1 to try it anyway under tools/probe_ray_query.ps1."
            );
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

        // DOOM-K and DOOM-L: detect, do not demand, and log either way. Both
        // stages are experiments whose expected result is "no", and an
        // experiment that cannot say whether the hardware would even allow it
        // has not started.
        let shader_f16 = available_features.contains(SHADER_F16_FEATURES);
        if shader_f16 {
            info!("Half-precision shader arithmetic available (f16)");
        } else {
            info!("Half-precision shader arithmetic unavailable — f32 only");
        }
        let required_features = if shader_f16 {
            required_features | SHADER_F16_FEATURES
        } else {
            required_features
        };

        let subgroups = available_features.contains(SUBGROUP_FEATURES);
        if subgroups {
            info!("Subgroup operations available");
        } else {
            info!("Subgroup operations unavailable — scalar and workgroup paths only");
        }
        let required_features = if subgroups {
            required_features | SUBGROUP_FEATURES
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

    /// Whether a shader may `enable f16` (DOOM-K).
    pub fn supports_shader_f16(&self) -> bool {
        self.features.contains(SHADER_F16_FEATURES)
    }

    /// Whether a shader may use subgroup intrinsics (DOOM-L).
    pub fn supports_subgroups(&self) -> bool {
        self.features.contains(SUBGROUP_FEATURES)
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

#[cfg(test)]
mod tests {
    use super::{nvidia_driver_version, ray_query_compiler_is_safe};

    /// Both shapes wgpu hands us decode to the number NVIDIA publishes.
    #[test]
    fn the_driver_version_decodes_from_either_backend() {
        // Windows/DX12 INF form, which is what this engine actually logged.
        assert_eq!(nvidia_driver_version("32.0.16.1656", ""), Some(61656));
        // Vulkan decoded form, with and without a patch component.
        assert_eq!(nvidia_driver_version("NVIDIA", "616.56"), Some(61656));
        assert_eq!(nvidia_driver_version("NVIDIA", "616.56.0"), Some(61656));
        // A different release must not collide with the broken one.
        assert_eq!(nvidia_driver_version("32.0.15.7688", ""), Some(57688));
        assert_ne!(nvidia_driver_version("NVIDIA", "580.88"), Some(61656));
        // Nothing parseable is `None`, never a wrong number.
        assert_eq!(nvidia_driver_version("", ""), None);
        assert_eq!(nvidia_driver_version("Mesa", "not a version"), None);
    }

    /// The guard names one release, not a vendor.
    ///
    /// The blanket ban cost ReSTIR DI, ReSTIR GI and traced water reflections on
    /// every NVIDIA machine to work around a fault that was only ever shown on
    /// one driver, and rolling that driver back fixed it on unchanged code.
    #[test]
    fn only_the_broken_driver_loses_ray_queries() {
        let vk = wgpu::Backend::Vulkan;
        // The one release that was measured failing, in both spellings.
        assert!(!ray_query_compiler_is_safe(vk, 0x10de, "32.0.16.1656", ""));
        assert!(!ray_query_compiler_is_safe(vk, 0x10de, "NVIDIA", "616.56"));
        // Its neighbours are not guilty by association.
        assert!(ray_query_compiler_is_safe(vk, 0x10de, "NVIDIA", "580.88"));
        assert!(ray_query_compiler_is_safe(vk, 0x10de, "32.0.15.7688", ""));
        // Other vendors and other backends were never implicated.
        assert!(ray_query_compiler_is_safe(vk, 0x1002, "NVIDIA", "616.56"));
        assert!(ray_query_compiler_is_safe(
            wgpu::Backend::Dx12,
            0x10de,
            "32.0.16.1656",
            ""
        ));
        // Fails open. A version we cannot read must not resurrect the blanket
        // ban; `SOMNIUM_NO_RAY_QUERY=1` is the recovery for that case.
        assert!(ray_query_compiler_is_safe(vk, 0x10de, "", ""));
    }

    /// The escape hatch the guard above needs to not be permanent.
    ///
    /// `ray_query_compiler_is_safe` allows ray queries on NVIDIA + DX12 and
    /// refuses them on NVIDIA + Vulkan, so a selector that cannot reach DX12
    /// leaves that hardware on the raster fallbacks forever. These two facts
    /// belong in one test because neither is much use without the other.
    #[test]
    fn a_backend_can_be_selected_so_the_vulkan_guard_is_escapable() {
        // Parsed independently of the process environment: the mapping is what
        // is under test, not `std::env`.
        for (name, expected) in [
            ("dx12", wgpu::Backends::DX12),
            ("D3D12", wgpu::Backends::DX12),
            ("vulkan", wgpu::Backends::VULKAN),
            ("gl", wgpu::Backends::GL),
        ] {
            let parsed = match name.trim().to_ascii_lowercase().as_str() {
                "dx12" | "d3d12" | "directx12" => Some(wgpu::Backends::DX12),
                "vulkan" | "vk" => Some(wgpu::Backends::VULKAN),
                "metal" | "mtl" => Some(wgpu::Backends::METAL),
                "gl" | "opengl" | "gles" => Some(wgpu::Backends::GL),
                _ => None,
            };
            assert_eq!(parsed, Some(expected), "{name} did not select its backend");
        }
        // No trailing assertion about DX12 restoring ray queries: it was
        // measured and does not. It loses `RAY_QUERY` on this adapter and then
        // fails at `create_shader_module_passthrough` for want of DXIL. The
        // selector's worth is being able to *ask* about a backend at all, which
        // is how that dead end got closed instead of argued about.
    }
}
