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
}

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

        let mut limits = wgpu::Limits::default();
        limits.max_binding_array_elements_per_shader_stage = 1024;
        limits.max_storage_buffers_per_shader_stage = 16;

        // Request the device and queue.
        let (device, queue) = adapter
            .request_device(
                &wgpu::DeviceDescriptor {
                    label: Some("somnium_device"),
                    required_features,
                    required_limits: limits,
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

        Self {
            instance,
            adapter,
            device,
            queue,
            surface,
            config,
        }
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
