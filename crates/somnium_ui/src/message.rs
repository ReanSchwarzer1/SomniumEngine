// Port of: example_repo/fyrox/Fyrox-master/fyrox-ui/src/message.rs
// Simplified: no routing strategy, no delivery mode, no OS event passthrough.
// Messages use Box<dyn Any> payload (downcast at the receiver).

use crate::pool::Handle;
use glam::Vec2;
use std::any::Any;

// Re-export so widget modules can import from here.
pub use winit::event::MouseButton;
pub use winit::keyboard::KeyCode;

/// Logical wheel delta produced by one platform `LineDelta` unit.
///
/// Routed wheel messages use logical pixels so scroll viewers and high-resolution
/// trackpads share one unit. Controls whose gesture is expressed in wheel lines
/// (such as graph zoom) divide by this value instead of treating pixels as
/// notches.
pub const WHEEL_DELTA_PER_LINE: f32 = 20.0;

/// Modifier keys captured at the OS-event boundary and delivered with input.
///
/// Keeping this on the message makes routed input self-contained: a widget can
/// implement range selection or a precision gesture without reaching back into
/// the editor shell's ambient state.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Modifiers {
    pub ctrl: bool,
    pub shift: bool,
    pub alt: bool,
    pub logo: bool,
}

impl Modifiers {
    /// The platform's primary shortcut modifier (Command on macOS, Ctrl
    /// elsewhere). Physical-Ctrl gestures should continue to read `ctrl`.
    #[inline]
    pub const fn command(self) -> bool {
        if cfg!(target_os = "macos") {
            self.logo
        } else {
            self.ctrl
        }
    }
}

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
    MouseDown {
        pos: Vec2,
        button: MouseButton,
        mods: Modifiers,
    },
    MouseUp {
        pos: Vec2,
        button: MouseButton,
        mods: Modifiers,
    },
    MouseMove {
        pos: Vec2,
        mods: Modifiers,
    },
    MouseWheel {
        pos: Vec2,
        /// Logical pixels. One OS line unit is [`WHEEL_DELTA_PER_LINE`].
        delta: f32,
        mods: Modifiers,
    },
    MouseEnter,
    MouseLeave,
    KeyDown(KeyCode, Modifiers),
    KeyUp(KeyCode, Modifiers),
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
            Self::MouseDown {
                pos,
                button,
                mods: Modifiers::default(),
            },
        )
    }
    pub fn mouse_up(dest: NodeHandle, pos: Vec2, button: MouseButton) -> UiMessage {
        UiMessage::new(
            dest,
            MessageDirection::FromWidget,
            Self::MouseUp {
                pos,
                button,
                mods: Modifiers::default(),
            },
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

/// Model refresh of mixed selection state; never an authored value change.
#[derive(Debug, Clone, Copy)]
pub struct MixedValue(pub bool);
