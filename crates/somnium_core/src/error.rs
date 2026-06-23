//! Engine-level error types.
//!
//! All fallible operations in `somnium_core` return [`EngineError`]. We use
//! [`thiserror`] to derive `std::error::Error` implementations, keeping
//! error messages descriptive and the crate's public API free of
//! third-party error types.

use thiserror::Error;

/// Top-level error type for the Somnium Engine core.
///
/// Each variant captures a distinct failure domain so that callers can
/// pattern-match on the category without parsing strings.
#[derive(Debug, Error)]
pub enum EngineError {
    /// Failed to create the OS window.
    ///
    /// This typically surfaces `winit::error::OsError` messages but is
    /// wrapped to avoid leaking `winit` types into the public API.
    #[error("Window creation failed: {0}")]
    WindowCreation(String),

    /// The platform event loop could not be started or encountered a
    /// fatal error during execution.
    #[error("Event loop error: {0}")]
    EventLoop(String),

    /// An invalid or inconsistent engine configuration was detected.
    #[error("Configuration error: {0}")]
    Config(String),
}
