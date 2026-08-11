// Port of: example_repo/fyrox/Fyrox-master/fyrox-ui/src/control.rs
// UiNode = widget base data + boxed Control implementation.
// Control trait provides the two-pass layout overrides and draw callback.
//
// LayoutCtx wraps a raw *mut UserInterface so containers can call
// measure_child / arrange_child during their overrides — same pattern as
// Fyrox passing `&UserInterface` to Control::measure_override.

use crate::{
    draw::DrawingContext,
    message::{NodeHandle, UiMessage},
    types::Rect,
    widget::Widget,
};
use glam::Vec2;
use std::ops::{Deref, DerefMut};

/// Context passed to Control::measure_override / arrange_override.
/// Provides access to child layout without a visible reference to UserInterface.
pub struct LayoutCtx {
    // Raw pointer — safe because we're single-threaded and we never hold a pool
    // borrow when calling into a control's measure/arrange overrides.
    pub(crate) ui_ptr: *mut crate::ui::UserInterface,
}

impl LayoutCtx {
    /// Measure a child node given available space. Returns child's desired size.
    pub fn measure_child(&mut self, handle: NodeHandle, available: Vec2) -> Vec2 {
        unsafe { (*self.ui_ptr).measure_node_pub(handle, available) }
    }

    /// Arrange (position + size) a child node within the given rect.
    pub fn arrange_child(&mut self, handle: NodeHandle, rect: Rect) {
        unsafe { (*self.ui_ptr).arrange_node_pub(handle, rect) }
    }

    /// Read a child's desired size (after measure).
    pub fn desired_size(&self, handle: NodeHandle) -> Vec2 {
        unsafe {
            (*self.ui_ptr)
                .nodes
                .try_borrow(handle.transmute())
                .map(|n| n.widget.desired_size)
                .unwrap_or_default()
        }
    }

    /// Read a child's desired local position (set by WidgetBuilder or animation).
    pub fn desired_local_position(&self, handle: NodeHandle) -> Vec2 {
        unsafe {
            (*self.ui_ptr)
                .nodes
                .try_borrow(handle.transmute())
                .map(|n| n.widget.desired_local_position)
                .unwrap_or_default()
        }
    }

    /// Read a child's grid row index.
    pub fn row(&self, handle: NodeHandle) -> usize {
        unsafe {
            (*self.ui_ptr)
                .nodes
                .try_borrow(handle.transmute())
                .map(|n| n.widget.row)
                .unwrap_or(0)
        }
    }

    /// Read a child's grid column index.
    pub fn column(&self, handle: NodeHandle) -> usize {
        unsafe {
            (*self.ui_ptr)
                .nodes
                .try_borrow(handle.transmute())
                .map(|n| n.widget.column)
                .unwrap_or(0)
        }
    }

    /// Measure text using the font atlas (no rasterization, uses font metrics only).
    /// Returns (total_advance_width, line_height) in logical pixels.
    pub fn measure_text(&self, text: &str, px: f32, font_id: u8) -> glam::Vec2 {
        unsafe {
            (*self.ui_ptr)
                .draw_ctx
                .font_atlas
                .measure_text(text, px, font_id)
        }
    }
}

/// The behavior interface every widget type must implement.
/// Analogous to Fyrox's `Control` trait in fyrox-ui/src/control.rs.
pub trait Control: Send + 'static {
    /// Bottom-up measure: return desired size given available space.
    /// Containers must call `ctx.measure_child()` for each child here.
    fn measure_override(&self, _widget: &Widget, _ctx: &mut LayoutCtx, available: Vec2) -> Vec2 {
        available
    }

    /// Top-down arrange: return actual size given final allocated size.
    /// Containers must call `ctx.arrange_child()` for each child here.
    fn arrange_override(&self, _widget: &Widget, _ctx: &mut LayoutCtx, final_size: Vec2) -> Vec2 {
        final_size
    }

    /// Emit draw commands for this widget's visual content.
    fn draw(&self, widget: &Widget, ctx: &mut DrawingContext) {
        ctx.push_rect_filled(widget.screen_bounds(), widget.background);
    }

    /// Handle a message routed to this widget (ToWidget direction).
    /// Push any outgoing (FromWidget) messages into `emit`; the caller drains it into the queue.
    fn handle_routed_message(
        &mut self,
        _widget: &mut Widget,
        _msg: &mut UiMessage,
        _emit: &mut Vec<UiMessage>,
    ) {
    }

    /// Returns true if this widget is a text-input type (TextBox, NumericField).
    /// When a text-input widget has focus, keyboard events are consumed by the UI
    /// instead of passing through to the game (WASD camera, gizmo shortcuts, etc.).
    fn is_text_input(&self) -> bool {
        false
    }
}

/// A node in the UI tree: layout base data + concrete widget behavior.
pub struct UiNode {
    pub widget: Widget,
    pub control: Box<dyn Control>,
}

impl UiNode {
    pub fn new(widget: Widget, control: Box<dyn Control>) -> Self {
        Self { widget, control }
    }
}

impl Deref for UiNode {
    type Target = Widget;
    fn deref(&self) -> &Widget {
        &self.widget
    }
}

impl DerefMut for UiNode {
    fn deref_mut(&mut self) -> &mut Widget {
        &mut self.widget
    }
}
