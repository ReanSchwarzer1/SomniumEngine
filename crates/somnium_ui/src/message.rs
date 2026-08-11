// Port of: example_repo/fyrox/Fyrox-master/fyrox-ui/src/message.rs
// Simplified: no routing strategy, no delivery mode, no OS event passthrough.
// Messages use Box<dyn Any> payload (downcast at the receiver).

use crate::pool::Handle;
use glam::Vec2;
use std::any::Any;

// Re-export so widget modules can import from here.
pub use winit::event::MouseButton;
pub use winit::keyboard::KeyCode;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MessageDirection {
    ToWidget,
    FromWidget,
}

impl MessageDirection {
    pub fn reverse(self) -> Self {
        match self {
            Self::ToWidget => Self::FromWidget,
            Self::FromWidget => Self::ToWidget,
        }
    }
}

/// Opaque marker type for node handles.
/// Pool<UiNode> uses Handle<UiNode> internally; call .transmute() to bridge.
#[derive(Debug)]
pub struct UiNodeTag;
pub type NodeHandle = Handle<UiNodeTag>;

pub struct UiMessage {
    pub handled: bool,
    pub destination: NodeHandle,
    pub direction: MessageDirection,
    pub data: Box<dyn Any + Send>,
}

impl UiMessage {
    pub fn new<T: Any + Send + 'static>(
        destination: NodeHandle,
        direction: MessageDirection,
        data: T,
    ) -> Self {
        Self {
            handled: false,
            destination,
            direction,
            data: Box::new(data),
        }
    }

    pub fn data<T: 'static>(&self) -> Option<&T> {
        self.data.downcast_ref::<T>()
    }
}

/// Core widget input/layout messages.
/// Port of: WidgetMessage in fyrox-ui/src/widget.rs
#[derive(Debug, Clone)]
pub enum WidgetMessage {
    MouseDown { pos: Vec2, button: MouseButton },
    MouseUp { pos: Vec2, button: MouseButton },
    MouseMove { pos: Vec2 },
    MouseWheel { pos: Vec2, delta: f32 },
    MouseEnter,
    MouseLeave,
    KeyDown(KeyCode),
    KeyUp(KeyCode),
    Text(String),
    Focus,
    Unfocus,
    Click,
    Visibility(bool),
    Enabled(bool),
    Width(f32),
    Height(f32),
    Remove,
}

impl WidgetMessage {
    pub fn mouse_down(dest: NodeHandle, pos: Vec2, button: MouseButton) -> UiMessage {
        UiMessage::new(
            dest,
            MessageDirection::FromWidget,
            Self::MouseDown { pos, button },
        )
    }
    pub fn mouse_up(dest: NodeHandle, pos: Vec2, button: MouseButton) -> UiMessage {
        UiMessage::new(
            dest,
            MessageDirection::FromWidget,
            Self::MouseUp { pos, button },
        )
    }
    pub fn click(dest: NodeHandle) -> UiMessage {
        UiMessage::new(dest, MessageDirection::FromWidget, Self::Click)
    }
}

/// Sent ToWidget to update displayed text content.
#[derive(Debug, Clone)]
pub enum TextMessage {
    SetText(String),
}

impl TextMessage {
    pub fn set_text(dest: NodeHandle, text: impl Into<String>) -> UiMessage {
        UiMessage::new(dest, MessageDirection::ToWidget, Self::SetText(text.into()))
    }
}
