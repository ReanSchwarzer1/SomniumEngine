//! Phase 11.5M: Output log capture.
//!
//! Implements a `tracing_subscriber::Layer` that forwards INFO/WARN/ERROR
//! events to a `mpsc::Sender<LogEntry>`.  The receiver is held by `Engine`
//! and drained every 5 frames; entries are forwarded to the HTML output log
//! via `UiManager::send_message("append_log", ...)`.
//!
//! Reference: Fyrox `editor/src/log.rs` — `LogSettings` ring buffer concept
//! adapted for the `tracing` crate.

use std::sync::mpsc::{channel, Receiver, Sender};
use tracing::{Level, Subscriber};
use tracing_subscriber::layer::Context;
use tracing_subscriber::Layer;

/// A single captured log entry.
pub struct LogEntry {
    /// "info" | "warn" | "error"
    pub level:   &'static str,
    /// The formatted log message text.
    pub message: String,
}

/// `tracing_subscriber::Layer` that captures events into a channel.
pub struct LogCaptureLayer {
    tx: Sender<LogEntry>,
}

impl<S: Subscriber> Layer<S> for LogCaptureLayer {
    fn on_event(&self, event: &tracing::Event<'_>, _ctx: Context<'_, S>) {
        let level = match *event.metadata().level() {
            Level::ERROR => "error",
            Level::WARN  => "warn",
            Level::INFO  => "info",
            _ => return,  // skip DEBUG / TRACE
        };
        let mut vis = MessageVisitor(String::new());
        event.record(&mut vis);
        if !vis.0.is_empty() {
            let _ = self.tx.send(LogEntry { level, message: vis.0 });
        }
    }
}

struct MessageVisitor(String);

impl tracing::field::Visit for MessageVisitor {
    fn record_str(&mut self, field: &tracing::field::Field, value: &str) {
        if field.name() == "message" {
            self.0 = value.to_string();
        }
    }

    fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn std::fmt::Debug) {
        if field.name() == "message" && self.0.is_empty() {
            self.0 = format!("{value:?}");
        }
    }
}

/// Create a `(Layer, Receiver)` pair.
///
/// Install the layer in the global subscriber; hand the receiver to `Engine`.
pub fn make_log_capture() -> (LogCaptureLayer, Receiver<LogEntry>) {
    let (tx, rx) = channel();
    (LogCaptureLayer { tx }, rx)
}
