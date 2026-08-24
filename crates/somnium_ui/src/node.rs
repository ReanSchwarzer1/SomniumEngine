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

    /// Whether a child is visible.
    ///
    /// A container that sizes itself to its content must skip hidden children,
    /// or a panel that hides one of two stacked states still reserves room for
    /// both.
    pub fn is_visible(&self, handle: NodeHandle) -> bool {
        unsafe {
            (*self.ui_ptr)
                .nodes
                .try_borrow(handle.transmute())
                .map(|n| n.widget.visibility)
                .unwrap_or(false)
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

    /// Screen-space bounds of any node (valid after arrange).
    pub fn screen_bounds(&self, handle: NodeHandle) -> Rect {
        unsafe {
            (*self.ui_ptr)
                .nodes
                .try_borrow(handle.transmute())
                .map(|n| n.widget.screen_bounds())
                .unwrap_or(Rect::ZERO)
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
        self.measure_text_tracked(text, px, font_id, 0.0)
    }

    /// [`measure_text`](Self::measure_text) with letter-spacing, so a tracked
    /// header measures the width it will actually draw.
    pub fn measure_text_tracked(
        &self,
        text: &str,
        px: f32,
        font_id: u8,
        tracking: f32,
    ) -> glam::Vec2 {
        unsafe {
            (*self.ui_ptr)
                .draw_ctx
                .font_atlas
                .measure_text_tracked(text, px, font_id, tracking)
        }
    }
}

/// Native mouse cursor to show while hovering a widget (Phase 26-I).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum CursorKind {
    #[default]
    Default,
    /// Clickable chrome (buttons, menus, swatches).
    Pointer,
    /// Text / numeric fields.
    Text,
    /// Vertical splitter bar (resize columns).
    ColResize,
    /// Horizontal splitter bar (resize rows).
    RowResize,
    /// Slider track or numeric slider.
    EwResize,
    /// Accepted drag whose operation preserves the source.
    Copy,
    /// Accepted drag whose operation relocates the source.
    Move,
    /// Drag target rejects the payload.
    NoDrop,
}

impl CursorKind {
    pub fn to_winit(self) -> winit::window::CursorIcon {
        use winit::window::CursorIcon;
        match self {
            Self::Default => CursorIcon::Default,
            Self::Pointer => CursorIcon::Pointer,
            Self::Text => CursorIcon::Text,
            Self::ColResize => CursorIcon::ColResize,
            Self::RowResize => CursorIcon::RowResize,
            Self::EwResize => CursorIcon::EwResize,
            Self::Copy => CursorIcon::Copy,
            Self::Move => CursorIcon::Move,
            Self::NoDrop => CursorIcon::NoDrop,
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

    /// Emit draw commands **after** this widget's children have drawn.
    ///
    /// `UserInterface::draw_node` paints a control and then recurses into its
    /// children, so anything a container needs to render *over* its content —
    /// a scroll-edge fade, a scrollbar that must not be covered — cannot go in
    /// [`Control::draw`]. Phase 27-G added this because the scroll fade did go
    /// there and was silently painted underneath the scrolled content: correct
    /// geometry, correct colour, invisible.
    ///
    /// Default is empty, so no existing widget changes behaviour.
    fn draw_over(&self, _widget: &Widget, _ctx: &mut DrawingContext) {}

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

    /// Cursor shown when the pointer is over this widget.
    fn cursor_icon(&self, _widget: &Widget, _pos: Vec2) -> CursorKind {
        CursorKind::Default
    }

    /// The value this widget is currently displaying, if it is a numeric
    /// control.
    ///
    /// Phase 26-Zeta-G needs to compare every inspector field against its
    /// baseline once per frame to decide which modified dots are lit. Reading
    /// the value back out of the tree keeps that in one place; the alternative
    /// was to mirror the value at each of the ~100 `set_value` call sites in
    /// `lib.rs` and hope none of them was ever missed.
    fn numeric_value(&self) -> Option<f32> {
        None
    }

    /// Whether this control is currently performing a modal-feeling pointer
    /// gesture. The UI owns the token; the control owns the state needed to
    /// restore itself if that token is cancelled.
    fn gesture_active(&self) -> bool {
        false
    }

    /// Cancel an active gesture, restoring its pre-gesture state. Any live
    /// restoration messages are appended to `emit` and follow normal routing.
    fn cancel_gesture(&mut self, _widget: &mut Widget, _emit: &mut Vec<UiMessage>) -> bool {
        false
    }

    /// Bounds which must remain visible while this node has keyboard focus.
    /// Composite controls such as TreeView override this with their focused
    /// row rather than returning the bounds of the entire control.
    fn focus_bounds(&self, widget: &Widget) -> Rect {
        widget.screen_bounds()
    }

    /// A control-level focus stop used by arrow traversal inside a region.
    fn is_keyboard_focusable(&self) -> bool {
        false
    }

    /// Scroll `target` into this control's viewport. Containers which are not
    /// scroll viewers leave this as a no-op.
    fn scroll_into_view(&mut self, _widget: &mut Widget, _target: Rect) -> bool {
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
