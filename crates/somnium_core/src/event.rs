//! Platform-independent engine events and translation layer.
//!
//! [`EngineEvent`] is our abstraction over raw OS/window events. Game
//! code should **only** interact with this enum — never with `winit`
//! types directly. This buys us:
//!
//! * **Testability** — synthetic events can be created in unit tests
//!   without a live window.
//! * **Portability** — swapping `winit` for another backend only
//!   requires changing [`translate_window_event`].
//! * **Serialisation** — events can be recorded to disk and replayed
//!   for deterministic debugging.
//!
//! # Translation
//!
//! The [`translate_window_event`] function performs a lossy conversion:
//! events that are irrelevant to game logic (e.g. `HoveredFile`,
//! `Ime`, `TouchpadPressure`) are discarded as `None`.

use winit::event::{ElementState, MouseScrollDelta, WindowEvent};
use winit::keyboard::{KeyCode, PhysicalKey};

/// Discriminant for press / release transitions.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum InputState {
    /// The key or button was pressed (or is being held).
    Pressed,
    /// The key or button was released.
    Released,
}

impl From<ElementState> for InputState {
    fn from(state: ElementState) -> Self {
        match state {
            ElementState::Pressed => Self::Pressed,
            ElementState::Released => Self::Released,
        }
    }
}

/// Engine-level event abstraction.
///
/// This enum is intentionally **non-exhaustive** (`#[non_exhaustive]`),
/// allowing future expansion without breaking downstream matches.
#[non_exhaustive]
#[derive(Debug, Clone)]
pub enum EngineEvent {
    /// The window's client area was resized.
    WindowResized {
        /// New width in physical pixels.
        width: u32,
        /// New height in physical pixels.
        height: u32,
    },

    /// The user (or OS) requested that the window be closed.
    WindowCloseRequested,

    /// The window gained or lost keyboard focus.
    WindowFocused(bool),

    /// A physical keyboard key was pressed or released.
    KeyInput {
        /// Physical key code (layout-independent).
        key: KeyCode,
        /// Whether the key was pressed or released.
        state: InputState,
    },

    /// The cursor moved within the window.
    CursorMoved {
        /// Cursor X position in logical pixels, relative to the
        /// top-left corner of the client area.
        x: f64,
        /// Cursor Y position in logical pixels.
        y: f64,
    },

    /// A mouse button was pressed or released.
    MouseButton {
        /// Which button (Left, Right, Middle, …).
        button: winit::event::MouseButton,
        /// Whether the button was pressed or released.
        state: InputState,
    },

    /// The scroll wheel (or trackpad) was scrolled.
    MouseWheel {
        /// Vertical scroll delta in logical "lines" (positive = up).
        delta_y: f64,
    },

    /// The application was suspended (e.g. minimised, or mobile
    /// backgrounding). The renderer should release GPU resources.
    Suspended,

    /// The application was resumed after a suspend. The renderer
    /// should re-acquire GPU resources.
    Resumed,

    /// Raw mouse movement delta (independent of cursor position).
    MouseMotion {
        /// Horizontal movement delta.
        delta_x: f32,
        /// Vertical movement delta.
        delta_y: f32,
    },

    /// A redraw was requested. The renderer should present a new frame.
    RedrawRequested,
}

/// Translate a raw [`winit::event::WindowEvent`] into an [`EngineEvent`].
///
/// Returns `None` for events that have no meaningful mapping in the
/// engine's event model (decorative events, IME, etc.).
///
/// # Design Note
///
/// We deliberately match on a **subset** of `WindowEvent` and discard
/// the rest. As we add subsystems (text input, drag-and-drop, touch),
/// we extend this function and the `EngineEvent` enum together.
#[must_use]
pub fn translate_window_event(event: &WindowEvent) -> Option<EngineEvent> {
    match event {
        WindowEvent::Resized(size) => Some(EngineEvent::WindowResized {
            width: size.width,
            height: size.height,
        }),

        WindowEvent::CloseRequested => Some(EngineEvent::WindowCloseRequested),

        WindowEvent::Focused(focused) => Some(EngineEvent::WindowFocused(*focused)),

        WindowEvent::KeyboardInput { event, .. } => {
            // We only care about physical keys for gameplay bindings.
            // Logical / text input will be handled separately.
            if let PhysicalKey::Code(key_code) = event.physical_key {
                Some(EngineEvent::KeyInput {
                    key: key_code,
                    state: event.state.into(),
                })
            } else {
                None
            }
        }

        WindowEvent::CursorMoved { position, .. } => Some(EngineEvent::CursorMoved {
            x: position.x,
            y: position.y,
        }),

        WindowEvent::MouseInput { state, button, .. } => Some(EngineEvent::MouseButton {
            button: *button,
            state: (*state).into(),
        }),

        WindowEvent::MouseWheel { delta, .. } => {
            let delta_y = match delta {
                MouseScrollDelta::LineDelta(_, y) => f64::from(*y),
                MouseScrollDelta::PixelDelta(pos) => pos.y,
            };
            Some(EngineEvent::MouseWheel { delta_y })
        }

        WindowEvent::RedrawRequested => Some(EngineEvent::RedrawRequested),

        // All other events are intentionally discarded.
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use winit::dpi::PhysicalSize;

    #[test]
    fn translate_resize() {
        let we = WindowEvent::Resized(PhysicalSize::new(1920, 1080));
        let eng = translate_window_event(&we).expect("should translate");
        match eng {
            EngineEvent::WindowResized { width, height } => {
                assert_eq!(width, 1920);
                assert_eq!(height, 1080);
            }
            _ => panic!("wrong variant"),
        }
    }

    #[test]
    fn translate_close() {
        let we = WindowEvent::CloseRequested;
        let eng = translate_window_event(&we).expect("should translate");
        assert!(matches!(eng, EngineEvent::WindowCloseRequested));
    }

    #[test]
    fn unknown_events_are_none() {
        // `Destroyed` is not mapped.
        let we = WindowEvent::Destroyed;
        assert!(translate_window_event(&we).is_none());
    }
}
