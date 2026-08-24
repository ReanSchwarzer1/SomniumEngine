//! Engine configuration.
//!
//! [`EngineConfig`] is the primary knob-panel for engine initialisation.
//! It is consumed once by [`Engine::run`](crate::app::Engine::run) and
//! thereafter stored as an immutable reference inside [`EngineContext`](crate::context::EngineContext).
//!
//! # Design Note
//!
//! All fields are public so that users can construct the config with
//! struct-literal syntax and override only the fields they care about:
//!
//! ```rust
//! use somnium_core::EngineConfig;
//!
//! let cfg = EngineConfig {
//!     window_title: "My Game".into(),
//!     ..Default::default()
//! };
//! ```

/// Immutable engine-wide configuration consumed at startup.
#[derive(Debug, Clone)]
pub struct EngineConfig {
    /// Project content directory. All inventory, authoring and preview-cache
    /// paths derive from this one source.
    pub content_root: std::path::PathBuf,
    /// Title displayed in the window's title bar.
    pub window_title: String,

    /// Initial window dimensions in logical pixels `(width, height)`.
    pub window_size: (u32, u32),

    /// Optional frame-rate cap. `None` means uncapped (run as fast as
    /// possible, typically limited only by vsync or GPU throughput).
    ///
    /// When set, the engine's frame limiter will sleep + spin-wait to
    /// hit the target cadence.
    pub target_fps: Option<u32>,

    /// Whether to request vertical sync from the graphics driver.
    ///
    /// When `true`, the swap chain will use `PresentMode::Fifo` (or the
    /// platform equivalent), capping the frame rate to the display
    /// refresh rate and eliminating tearing.
    pub vsync: bool,

    /// Whether the user may resize the window at runtime.
    pub resizable: bool,
}

impl Default for EngineConfig {
    /// Sensible defaults for a desktop windowed application.
    ///
    /// | Field          | Default              |
    /// |----------------|----------------------|
    /// | `window_title` | `"Somnium Engine"`   |
    /// | `window_size`  | `1280 × 720`         |
    /// | `target_fps`   | `Some(60)`           |
    /// | `vsync`        | `true`               |
    /// | `resizable`    | `true`               |
    fn default() -> Self {
        Self {
            content_root: std::env::var_os("SOMNIUM_CONTENT_ROOT")
                .map(std::path::PathBuf::from)
                .unwrap_or_else(|| std::path::PathBuf::from("assets")),
            window_title: "Somnium Engine".into(),
            window_size: (1280, 720),
            target_fps: Some(60),
            vsync: true,
            resizable: true,
        }
    }
}
