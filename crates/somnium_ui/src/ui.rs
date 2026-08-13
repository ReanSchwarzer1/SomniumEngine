// Port of: example_repo/fyrox/Fyrox-master/fyrox-ui/src/lib.rs
// Two-pass layout engine (measure + arrange), hit-test, message queue.
// All Fyrox reflection, styles, animations, drag-drop, and multi-thread machinery removed.
//
// Handle bridging:
//   Pool<UiNode> returns Handle<UiNode> (internal = IH)
//   Public API uses NodeHandle = Handle<UiNodeTag> (opaque)
//   Bridge: .transmute::<UiNode>() on incoming, .transmute::<UiNodeTag>() on outgoing.
//
// Layout safety:
//   Control::measure_override / arrange_override receive a LayoutCtx with *mut UserInterface.
//   Before calling the control, all pool borrows are released. The raw pointer is valid for
//   the call duration (single-threaded, no aliased mutable borrows).

use crate::{
    draw::DrawingContext,
    message::{MessageDirection, NodeHandle, UiMessage, WidgetMessage},
    node::{Control, LayoutCtx, UiNode},
    pool::Pool,
    types::{HorizontalAlignment, Rect, VerticalAlignment},
    widget::Widget,
};
use glam::Vec2;
use std::collections::VecDeque;
use winit::event::{ElementState, MouseScrollDelta, WindowEvent};
use winit::keyboard::PhysicalKey;

pub type NodePool = Pool<UiNode>;

// Internal concrete handle type (what Pool<UiNode> uses).
type IH = crate::pool::Handle<UiNode>;

#[inline]
fn to_ih(h: NodeHandle) -> IH {
    h.transmute()
}
#[inline]
fn to_nh(h: IH) -> NodeHandle {
    h.transmute()
}

pub struct UserInterface {
    pub nodes: NodePool,
    root_ih: IH,
    pub screen_size: Vec2,
    message_queue: VecDeque<UiMessage>,
    pub draw_ctx: DrawingContext,
    pub cursor_pos: Vec2,
    focused_ih: IH,
    #[allow(dead_code)]
    captured_ih: IH,
    /// Handle of the viewport area; mouse events here pass through to the game.
    pub viewport_handle: NodeHandle,
}

impl UserInterface {
    pub fn new(screen_w: f32, screen_h: f32) -> Self {
        let mut nodes: NodePool = Pool::new();
        let root_widget = {
            let mut w = Widget::default();
            w.name = "Root".into();
            w.width = screen_w;
            w.height = screen_h;
            // clip_bounds must be initialised to the screen rect so the first
            // arrange pass produces a non-zero clip for all descendants.
            w.clip_bounds = crate::types::Rect::new(0.0, 0.0, screen_w, screen_h);
            w
        };
        let root_ih = nodes.spawn(UiNode::new(root_widget, Box::new(RootControl)));
        Self {
            nodes,
            root_ih,
            screen_size: Vec2::new(screen_w, screen_h),
            message_queue: VecDeque::new(),
            draw_ctx: DrawingContext::new(screen_w, screen_h),
            cursor_pos: Vec2::ZERO,
            focused_ih: IH::NONE,
            captured_ih: IH::NONE,
            viewport_handle: NodeHandle::NONE,
        }
    }

    pub fn root(&self) -> NodeHandle {
        to_nh(self.root_ih)
    }

    /// Set the viewport handle so mouse events in the viewport area pass through to the game.
    pub fn set_viewport_handle(&mut self, handle: NodeHandle) {
        self.viewport_handle = handle;
    }

    /// Returns true if a text-input widget (TextBox or NumericField) currently has keyboard focus.
    /// Used to decide whether keyboard events should be consumed by the UI or pass through to the game.
    pub fn has_text_focus(&self) -> bool {
        if !self.focused_ih.is_some() {
            return false;
        }
        if let Ok(node) = self.nodes.try_borrow(self.focused_ih) {
            return node.control.is_text_input();
        }
        false
    }

    // -----------------------------------------------------------------------
    // Node management
    // -----------------------------------------------------------------------

    pub fn add_node(&mut self, mut node: UiNode, parent: NodeHandle) -> NodeHandle {
        let parent_ih = to_ih(parent);
        node.widget.parent = parent;
        // Initialise clip_bounds to the full screen so the first arrange pass
        // computes a correct non-zero clip rect for every node.
        node.widget.clip_bounds =
            crate::types::Rect::new(0.0, 0.0, self.screen_size.x, self.screen_size.y);
        let handle_ih = self.nodes.spawn(node);
        let handle_nh = to_nh(handle_ih);
        // BUG-010 fix: set widget.handle so that FromWidget messages (Button/Menu clicks)
        // have the correct destination handle. Without this, all emit() calls use NONE.
        if let Ok(n) = self.nodes.try_borrow_mut(handle_ih) {
            n.widget.handle = handle_nh;
        }
        if let Ok(p) = self.nodes.try_borrow_mut(parent_ih) {
            p.widget.children.push(handle_nh);
            p.widget.invalidate_layout();
        }
        // Propagate invalidation up so ancestors don't short-circuit measure/arrange.
        self.invalidate_ancestors(parent);
        handle_nh
    }

    pub fn remove_node(&mut self, handle: NodeHandle) {
        let handle_ih = to_ih(handle);
        let children: Vec<NodeHandle> = self
            .nodes
            .try_borrow(handle_ih)
            .map(|n| n.widget.children.clone())
            .unwrap_or_default();
        for ch in children {
            self.remove_node(ch);
        }
        // Read parent_nh before any mutable borrow to avoid aliasing conflict.
        let parent_nh = self
            .nodes
            .try_borrow(handle_ih)
            .map(|n| n.widget.parent)
            .unwrap_or(NodeHandle::NONE);
        if let Ok(p) = self.nodes.try_borrow_mut(to_ih(parent_nh)) {
            p.widget.children.retain(|&h| h != handle);
            p.widget.invalidate_layout();
        }
        let _ = self.nodes.try_free(handle_ih);
        // Propagate invalidation up so ancestors remeasure after child removal.
        if parent_nh.is_some() {
            self.invalidate_ancestors(parent_nh);
        }
    }

    /// Walk from `start` toward the root, invalidating each ancestor's layout.
    pub fn invalidate_ancestors(&mut self, start: NodeHandle) {
        let mut current = start;
        loop {
            let ih = to_ih(current);
            let parent = match self.nodes.try_borrow_mut(ih) {
                Ok(node) => {
                    node.widget.measure_valid = false;
                    node.widget.arrange_valid = false;
                    node.widget.parent
                }
                Err(_) => return,
            };
            if !parent.is_some() {
                return;
            }
            current = parent;
        }
    }

    // -----------------------------------------------------------------------
    // Public layout entry points (called by LayoutCtx from within Control impls)
    // -----------------------------------------------------------------------

    pub(crate) fn measure_node_pub(&mut self, handle: NodeHandle, available: Vec2) -> Vec2 {
        self.measure_node(to_ih(handle), available)
    }

    pub(crate) fn arrange_node_pub(&mut self, handle: NodeHandle, rect: Rect) {
        self.arrange_node(to_ih(handle), rect)
    }

    // -----------------------------------------------------------------------
    // Layout — two-pass: measure (bottom-up) then arrange (top-down)
    // Port of: UserInterface::measure_node / arrange_node in fyrox-ui/src/lib.rs
    // -----------------------------------------------------------------------

    pub fn perform_layout(&mut self) {
        let screen = self.screen_size;
        let root = self.root_ih;
        self.measure_node(root, screen);
        let root_rect = Rect::new(0.0, 0.0, screen.x, screen.y);
        self.arrange_node(root, root_rect);
    }

    pub(crate) fn measure_node(&mut self, handle: IH, available: Vec2) -> Vec2 {
        // --- Read snapshot (releases borrow before calling control) ---
        let snap = match self.nodes.try_borrow(handle) {
            Ok(n) => WidgetSnap {
                measure_valid: n.widget.measure_valid,
                prev_measure: n.widget.prev_measure,
                visibility: n.widget.visibility,
                desired_size: n.widget.desired_size,
                margin: n.widget.margin,
                width: n.widget.width,
                height: n.widget.height,
                min_size: n.widget.min_size,
                max_size: n.widget.max_size,
            },
            Err(_) => return Vec2::ZERO,
        };

        if snap.measure_valid && snap.prev_measure == available {
            return snap.desired_size;
        }
        if !snap.visibility {
            return Vec2::ZERO;
        }

        let margin = snap.margin;
        let inner = (available - Vec2::new(margin.h(), margin.v())).max(Vec2::ZERO);

        // --- Call control.measure_override (no pool borrows held) ---
        let widget_snap_copy = snap; // move; borrow is gone
        let desired = {
            // Build layout input: respect explicit width/height from widget.
            let constrained_inner = Vec2::new(
                if widget_snap_copy.width.is_nan() {
                    inner.x
                } else {
                    widget_snap_copy.width
                },
                if widget_snap_copy.height.is_nan() {
                    inner.y
                } else {
                    widget_snap_copy.height
                },
            );
            let constrained =
                constrained_inner.clamp(widget_snap_copy.min_size, widget_snap_copy.max_size);

            // Get raw pointers — pool borrow already released.
            let (widget_ptr, control_ptr) = {
                let node = self.nodes.borrow_mut(handle);
                let w = &node.widget as *const Widget;
                let c = node.control.as_ref() as *const dyn Control;
                (w, c)
            };
            let mut ctx = LayoutCtx {
                ui_ptr: self as *mut Self,
            };
            // SAFETY: widget_ptr is stable in pool record; ctx has raw ptr not a borrow.
            // control.measure_override may call ctx.measure_child for other handles.
            let mut raw =
                unsafe { (*control_ptr).measure_override(&*widget_ptr, &mut ctx, constrained) };

            // Apply explicit size overrides.
            if !widget_snap_copy.width.is_nan() {
                raw.x = widget_snap_copy.width;
            }
            if !widget_snap_copy.height.is_nan() {
                raw.y = widget_snap_copy.height;
            }
            raw.clamp(widget_snap_copy.min_size, widget_snap_copy.max_size)
        };

        // Desired includes margin.
        let desired_with_margin = desired + Vec2::new(margin.h(), margin.v());

        // --- Commit ---
        let node = self.nodes.borrow_mut(handle);
        node.widget.prev_measure = available;
        node.widget.desired_size = desired_with_margin;
        node.widget.measure_valid = true;
        desired_with_margin
    }

    fn get_parent_clip(&self, handle: IH) -> Rect {
        let parent_nh = match self.nodes.try_borrow(handle) {
            Ok(n) => n.widget.parent,
            Err(_) => return Rect::INF,
        };
        if parent_nh.is_some() {
            if let Ok(p) = self.nodes.try_borrow(to_ih(parent_nh)) {
                return p.widget.clip_bounds;
            }
        }
        // If no parent (e.g. root), return an unconstrained rect that covers negative and positive space.
        Rect::new(-1e5, -1e5, 2e5, 2e5)
    }

    pub(crate) fn arrange_node(&mut self, handle: IH, final_rect: Rect) {
        let parent_clip = self.get_parent_clip(handle);

        // --- Read snapshot ---
        let snap = match self.nodes.try_borrow(handle) {
            Ok(n) => ArrangeSnap {
                arrange_valid: n.widget.arrange_valid,
                prev_arrange: n.widget.prev_arrange,
                visibility: n.widget.visibility,
                margin: n.widget.margin,
                h_align: n.widget.horizontal_alignment,
                v_align: n.widget.vertical_alignment,
                desired_size: n.widget.desired_size,
                width: n.widget.width,
                height: n.widget.height,
                min_size: n.widget.min_size,
                max_size: n.widget.max_size,
                clip_to_bounds: n.widget.clip_to_bounds,
            },
            Err(_) => return,
        };

        if snap.arrange_valid && snap.prev_arrange == final_rect {
            return;
        }
        if !snap.visibility {
            return;
        }

        let margin = snap.margin;
        let avail = Vec2::new(
            (final_rect.w - margin.h()).max(0.0),
            (final_rect.h - margin.v()).max(0.0),
        );
        let desired_inner = snap.desired_size - Vec2::new(margin.h(), margin.v());

        let mut size = avail;
        if snap.h_align != HorizontalAlignment::Stretch {
            size.x = size.x.min(desired_inner.x);
        }
        if snap.v_align != VerticalAlignment::Stretch {
            size.y = size.y.min(desired_inner.y);
        }
        if !snap.width.is_nan() {
            size.x = snap.width;
        }
        if !snap.height.is_nan() {
            size.y = snap.height;
        }
        size = size.clamp(snap.min_size, snap.max_size);
        size.x = size.x.ceil();
        size.y = size.y.ceil();

        let mut origin = Vec2::new(final_rect.x + margin.left, final_rect.y + margin.top);
        match snap.h_align {
            HorizontalAlignment::Center | HorizontalAlignment::Stretch => {
                origin.x += (avail.x - size.x) * 0.5;
            }
            HorizontalAlignment::Right => origin.x += avail.x - size.x,
            HorizontalAlignment::Left => {}
        }
        match snap.v_align {
            VerticalAlignment::Center | VerticalAlignment::Stretch => {
                origin.y += (avail.y - size.y) * 0.5;
            }
            VerticalAlignment::Bottom => origin.y += avail.y - size.y,
            VerticalAlignment::Top => {}
        }
        origin.x = origin.x.floor();
        origin.y = origin.y.floor();

        let node_rect = Rect::from_pos_size(origin, size);
        let node_clip = if snap.clip_to_bounds {
            node_rect.intersect(&parent_clip)
        } else {
            parent_clip
        };

        // --- Call control.arrange_override ---
        let actual_size = {
            let (widget_ptr, control_ptr) = {
                let node = self.nodes.borrow_mut(handle);
                node.widget.actual_local_position = origin;
                node.widget.actual_local_size = size;
                node.widget.clip_bounds = node_clip;
                node.widget.prev_arrange = final_rect;
                node.widget.arrange_valid = true;
                let w = &node.widget as *const Widget;
                let c = node.control.as_ref() as *const dyn Control;
                (w, c)
            };
            let mut ctx = LayoutCtx {
                ui_ptr: self as *mut Self,
            };
            // SAFETY: see measure_node. control.arrange_override calls ctx.arrange_child
            // for child handles — no aliased borrows.
            unsafe { (*control_ptr).arrange_override(&*widget_ptr, &mut ctx, size) }
        };

        // Update actual size if control returned a different size.
        if actual_size != size {
            let node = self.nodes.borrow_mut(handle);
            node.widget.actual_local_size = actual_size;
        }
    }

    // -----------------------------------------------------------------------
    // Hit testing
    // -----------------------------------------------------------------------

    pub fn hit_test(&self, point: Vec2) -> NodeHandle {
        to_nh(self.pick_node(self.root_ih, point))
    }

    /// Cursor for the widget under the pointer (or the captured widget while dragging).
    pub fn cursor_kind(&self) -> crate::node::CursorKind {
        let handle = if self.captured_ih.is_some() {
            to_nh(self.captured_ih)
        } else {
            self.hit_test(self.cursor_pos)
        };
        if handle.is_none() || handle == self.viewport_handle {
            return crate::node::CursorKind::Default;
        }
        let Ok(node) = self.nodes.try_borrow(to_ih(handle)) else {
            return crate::node::CursorKind::Default;
        };
        node.control.cursor_icon(&node.widget, self.cursor_pos)
    }

    fn pick_node(&self, handle: IH, pt: Vec2) -> IH {
        let node = match self.nodes.try_borrow(handle) {
            Ok(n) => n,
            Err(_) => return IH::NONE,
        };
        if !node.widget.global_visibility
            || !node.widget.hit_test_visibility
            || !node.widget.enabled
        {
            return IH::NONE;
        }
        let children = node.widget.children.clone();
        let _ = node;
        for ch_nh in children.iter().rev() {
            let p = self.pick_node(to_ih(*ch_nh), pt);
            if p.is_some() {
                return p;
            }
        }
        let node = match self.nodes.try_borrow(handle) {
            Ok(n) => n,
            Err(_) => return IH::NONE,
        };
        if node.widget.clip_bounds.contains(pt) && node.widget.screen_bounds().contains(pt) {
            handle
        } else {
            IH::NONE
        }
    }

    // -----------------------------------------------------------------------
    // Focus / capture
    // -----------------------------------------------------------------------

    pub fn focused(&self) -> NodeHandle {
        to_nh(self.focused_ih)
    }

    pub fn set_focus(&mut self, handle: NodeHandle) {
        self.focused_ih = to_ih(handle);
    }

    /// Tooltip string on the widget under `pos`, walking parents if empty.
    pub fn tooltip_at(&self, pos: Vec2) -> String {
        let mut h = self.hit_test(pos);
        while h.is_some() {
            if let Ok(n) = self.nodes.try_borrow(to_ih(h)) {
                if !n.widget.tooltip.is_empty() {
                    return n.widget.tooltip.clone();
                }
                h = n.widget.parent;
            } else {
                break;
            }
        }
        String::new()
    }

    // -----------------------------------------------------------------------
    // Message queue
    // -----------------------------------------------------------------------

    pub fn send(&mut self, msg: UiMessage) {
        self.message_queue.push_back(msg);
    }

    pub fn poll_message(&mut self) -> Option<UiMessage> {
        self.message_queue.pop_front()
    }

    pub fn update(&mut self) -> Vec<UiMessage> {
        let mut outgoing = Vec::new();
        while let Some(mut msg) = self.message_queue.pop_front() {
            match msg.direction {
                MessageDirection::ToWidget => {
                    // Deliver to destination widget, then bubble up to ancestors.
                    // This matches Fyrox's bubble_message pattern: child → parent → grandparent.
                    let mut current_ih = to_ih(msg.destination);
                    loop {
                        let mut emit = Vec::new();
                        let parent_nh = if let Ok(node) = self.nodes.try_borrow_mut(current_ih) {
                            let widget_ptr = &mut node.widget as *mut Widget;
                            let control_ptr = node.control.as_mut() as *mut dyn Control;
                            unsafe {
                                (*control_ptr).handle_routed_message(
                                    &mut *widget_ptr,
                                    &mut msg,
                                    &mut emit,
                                );
                            }
                            node.widget.parent
                        } else {
                            break;
                        };
                        for e in emit {
                            self.message_queue.push_back(e);
                        }
                        // If handled or no parent, stop bubbling.
                        if msg.handled || !parent_nh.is_some() {
                            break;
                        }
                        current_ih = to_ih(parent_nh);
                    }
                }
                MessageDirection::FromWidget => {
                    outgoing.push(msg);
                }
            }
        }
        outgoing
    }

    // -----------------------------------------------------------------------
    // Draw
    // -----------------------------------------------------------------------

    pub fn draw(&mut self) {
        self.draw_ctx.clear(self.screen_size.x, self.screen_size.y);
        self.update_global_visibility(self.root_ih, true);
        self.draw_node(self.root_ih);
    }

    fn update_global_visibility(&mut self, handle: IH, parent_visible: bool) {
        let children = match self.nodes.try_borrow_mut(handle) {
            Ok(node) => {
                node.widget.global_visibility = parent_visible && node.widget.visibility;
                node.widget.children.clone()
            }
            Err(_) => return,
        };
        let gv = self
            .nodes
            .try_borrow(handle)
            .map(|n| n.widget.global_visibility)
            .unwrap_or(false);
        for ch in children {
            self.update_global_visibility(to_ih(ch), gv);
        }
    }

    fn draw_node(&mut self, handle: IH) {
        let (clip, children) = match self.nodes.try_borrow(handle) {
            Ok(n) if n.widget.global_visibility => {
                (n.widget.clip_bounds, n.widget.children.clone())
            }
            _ => return,
        };
        self.draw_ctx.push_clip_rect(clip);
        {
            let node = self.nodes.borrow_mut(handle);
            let widget_ptr = &node.widget as *const Widget;
            let control_ptr = node.control.as_ref() as *const dyn Control;
            // SAFETY: widget lives in pool record, draw_ctx is a separate allocation.
            unsafe {
                (*control_ptr).draw(&*widget_ptr, &mut self.draw_ctx);
            }
        }
        for ch in children {
            self.draw_node(to_ih(ch));
        }
        self.draw_ctx.pop_clip_rect();
    }

    // -----------------------------------------------------------------------
    // Resize
    // -----------------------------------------------------------------------

    // -----------------------------------------------------------------------
    // Font management
    // -----------------------------------------------------------------------

    /// Load a TrueType/OpenType font from raw bytes.
    /// Returns the font_id to pass to TextBuilder/push_text.
    pub fn add_font(&mut self, bytes: &[u8]) -> Result<u8, &'static str> {
        self.draw_ctx.font_atlas.add_font(bytes)
    }

    // -----------------------------------------------------------------------
    // Resize
    // -----------------------------------------------------------------------

    pub fn resize(&mut self, w: f32, h: f32) {
        self.screen_size = Vec2::new(w, h);
        // Update root widget dimensions.
        if let Ok(root) = self.nodes.try_borrow_mut(self.root_ih) {
            root.widget.width = w;
            root.widget.height = h;
            root.widget.clip_bounds = crate::types::Rect::new(0.0, 0.0, w, h);
        }
        // BUG-009 fix: Invalidate ALL nodes in the tree, not just root.
        // Without this, child widgets retain stale cached layouts after resize.
        self.invalidate_all();
    }

    /// Invalidate measure_valid and arrange_valid on every node in the tree.
    /// Called on resize to force a complete re-layout.
    fn invalidate_all(&mut self) {
        for (_h, node) in self.nodes.pair_iter_mut() {
            node.widget.measure_valid = false;
            node.widget.arrange_valid = false;
        }
    }

    // -----------------------------------------------------------------------
    // Utility mutations
    // -----------------------------------------------------------------------

    /// Remove all children of a node (recursive).
    pub fn clear_children(&mut self, handle: NodeHandle) {
        let children = self
            .nodes
            .try_borrow(to_ih(handle))
            .map(|n| n.widget.children.clone())
            .unwrap_or_default();
        for ch in children {
            self.remove_node(ch);
        }
    }

    pub fn first_child(&self, handle: NodeHandle) -> NodeHandle {
        self.nodes
            .try_borrow(to_ih(handle))
            .ok()
            .and_then(|n| n.widget.children.first().copied())
            .unwrap_or(NodeHandle::NONE)
    }

    pub fn set_desired_position(&mut self, handle: NodeHandle, pos: Vec2) {
        if let Ok(node) = self.nodes.try_borrow_mut(to_ih(handle)) {
            node.widget.desired_local_position = pos;
            node.widget.measure_valid = false;
            node.widget.arrange_valid = false;
        }
    }

    /// Show or hide a widget and invalidate layout.
    pub fn set_visibility(&mut self, handle: NodeHandle, visible: bool) {
        if let Ok(node) = self.nodes.try_borrow_mut(to_ih(handle)) {
            if node.widget.visibility != visible {
                node.widget.visibility = visible;
                node.widget.invalidate_layout();
            }
        }
        // MUST invalidate ancestors, otherwise root will skip measure/arrange and the child stays un-arranged!
        self.invalidate_ancestors(handle);
    }

    // -----------------------------------------------------------------------
    // OS event routing
    // -----------------------------------------------------------------------

    /// Route a winit WindowEvent into the widget tree.
    /// Returns true if the UI consumed the event (caller should not forward to game/gizmo).
    ///
    /// Consumption rules (matching Fyrox's `process_os_event` pattern):
    /// - CursorMoved: NEVER consumed (both UI hover and game camera need it)
    /// - MouseInput: consumed only if cursor is over an opaque UI widget (not viewport)
    /// - KeyboardInput: consumed only if a text-input widget (TextBox/NumericField) has focus
    /// - RMB: always passes through for camera look
    pub fn process_os_event(&mut self, event: &winit::event::WindowEvent) -> bool {
        if self.nodes.try_borrow(self.root_ih).is_err() {
            return false;
        }

        match event {
            WindowEvent::CursorMoved { position, .. } => {
                self.cursor_pos = Vec2::new(position.x as f32, position.y as f32);
                // While a widget holds the mouse (a slider being dragged), it
                // keeps receiving moves even when the cursor leaves its bounds.
                let captured = to_nh(self.captured_ih);
                if captured.is_some() {
                    let pos = self.cursor_pos;
                    self.send(UiMessage::new(
                        captured,
                        MessageDirection::ToWidget,
                        WidgetMessage::MouseMove { pos },
                    ));
                }
                false // Never consumed — both UI and game need cursor tracking
            }

            WindowEvent::MouseInput { state, button, .. } => {
                let pos = self.cursor_pos;
                let hit = self.hit_test(pos);
                // RMB over the viewport is fly-cam. RMB over chrome can open
                // context menus (Phase 26-B).
                if *button == winit::event::MouseButton::Right {
                    let over_viewport = !hit.is_some() || hit == self.viewport_handle;
                    if over_viewport {
                        return false;
                    }
                }

                // Check if the hit widget is a viewport area (transparent/non-interactive)
                let over_viewport = !hit.is_some() || hit == self.viewport_handle;

                if over_viewport {
                    // Unfocus any active text widget when clicking viewport
                    if matches!(state, ElementState::Pressed) {
                        let old = to_nh(self.focused_ih);
                        if old.is_some() {
                            self.send(UiMessage::new(
                                old,
                                MessageDirection::ToWidget,
                                WidgetMessage::Unfocus,
                            ));
                            self.focused_ih = IH::NONE;
                        }
                    }
                    return false; // Let the game handle viewport clicks
                }

                if matches!(state, ElementState::Pressed) {
                    let old = to_nh(self.focused_ih);
                    if old != hit && old.is_some() {
                        self.send(UiMessage::new(
                            old,
                            MessageDirection::ToWidget,
                            WidgetMessage::Unfocus,
                        ));
                    }
                    self.focused_ih = to_ih(hit);
                    // Focus is re-sent even when this widget already held it. A
                    // numeric field that was drag-scrubbed drops its edit state
                    // while staying the focused node, so skipping the message
                    // here would leave it impossible to click back into typing.
                    // Re-focusing an already-focused field also re-selects its
                    // contents, which is what clicking one should do anyway.
                    self.send(UiMessage::new(
                        hit,
                        MessageDirection::ToWidget,
                        WidgetMessage::Focus,
                    ));
                }

                // Press captures the mouse; release delivers to the capturing
                // widget rather than whatever is under the cursor. Without this
                // a widget that was pressed and then released elsewhere never
                // sees its MouseUp and stays stuck in the pressed state.
                let (target, wmsg) = match state {
                    ElementState::Pressed => {
                        self.captured_ih = to_ih(hit);
                        (
                            hit,
                            WidgetMessage::MouseDown {
                                pos,
                                button: *button,
                            },
                        )
                    }
                    ElementState::Released => {
                        let captured = to_nh(self.captured_ih);
                        self.captured_ih = IH::NONE;
                        let target = if captured.is_some() { captured } else { hit };
                        (
                            target,
                            WidgetMessage::MouseUp {
                                pos,
                                button: *button,
                            },
                        )
                    }
                };
                self.send(UiMessage::new(target, MessageDirection::ToWidget, wmsg));
                true
            }

            WindowEvent::MouseWheel { delta, .. } => {
                let pos = self.cursor_pos;
                let hit = self.hit_test(pos);
                let over_viewport = !hit.is_some() || hit == self.viewport_handle;
                if over_viewport {
                    return false;
                }
                if hit.is_some() {
                    let d = match delta {
                        MouseScrollDelta::LineDelta(_, y) => *y * 20.0,
                        MouseScrollDelta::PixelDelta(p) => p.y as f32,
                    };
                    self.send(UiMessage::new(
                        hit,
                        MessageDirection::ToWidget,
                        WidgetMessage::MouseWheel { pos, delta: d },
                    ));
                    return true;
                }
                false
            }

            WindowEvent::KeyboardInput { event: key_ev, .. } => {
                // Only consume keyboard events if a text-input widget has focus
                if !self.has_text_focus() {
                    return false;
                }
                let focused_ih = self.focused_ih;
                if focused_ih.is_some() {
                    let focused = to_nh(focused_ih);
                    if let PhysicalKey::Code(code) = key_ev.physical_key {
                        let wmsg = if key_ev.state == ElementState::Pressed {
                            WidgetMessage::KeyDown(code)
                        } else {
                            WidgetMessage::KeyUp(code)
                        };
                        self.send(UiMessage::new(focused, MessageDirection::ToWidget, wmsg));
                    }
                    if key_ev.state == ElementState::Pressed {
                        if let Some(text) = &key_ev.text {
                            let s = text.to_string();
                            if !s.is_empty() && !s.chars().all(|c| c.is_control()) {
                                self.send(UiMessage::new(
                                    focused,
                                    MessageDirection::ToWidget,
                                    WidgetMessage::Text(s),
                                ));
                            }
                        }
                    }
                    return true;
                }
                false
            }

            _ => false,
        }
    }
}

// ---------------------------------------------------------------------------
// Helper snapshot structs (avoids holding borrows across control calls)
// ---------------------------------------------------------------------------
struct WidgetSnap {
    measure_valid: bool,
    prev_measure: Vec2,
    visibility: bool,
    desired_size: Vec2,
    margin: crate::types::Thickness,
    width: f32,
    height: f32,
    min_size: Vec2,
    max_size: Vec2,
}

struct ArrangeSnap {
    arrange_valid: bool,
    prev_arrange: Rect,
    visibility: bool,
    margin: crate::types::Thickness,
    h_align: HorizontalAlignment,
    v_align: VerticalAlignment,
    desired_size: Vec2,
    width: f32,
    height: f32,
    min_size: Vec2,
    max_size: Vec2,
    clip_to_bounds: bool,
}

// ---------------------------------------------------------------------------
// Root control — transparent canvas (children position themselves absolutely).
// ---------------------------------------------------------------------------
struct RootControl;
impl Control for RootControl {
    fn measure_override(&self, widget: &Widget, ctx: &mut LayoutCtx, available: Vec2) -> Vec2 {
        // Measure children with the screen size so Grid stretch columns resolve correctly.
        // Popup children have explicit width/height so they're unaffected.
        for &ch in &widget.children {
            ctx.measure_child(ch, available);
        }
        // Return available (screen size) — not Vec2::ZERO — so the root's desired_size
        // matches the screen, and arrange_node receives the correct rect.
        available
    }

    fn arrange_override(&self, widget: &Widget, ctx: &mut LayoutCtx, final_size: Vec2) -> Vec2 {
        // Arrange ALL children with the full screen rect, not their desired size.
        // The outer Grid needs the full window dimensions to compute stretch columns
        // (e.g. viewport = screen_width - 40 - 280).
        for &ch in &widget.children {
            let pos = ctx.desired_local_position(ch);
            ctx.arrange_child(ch, Rect::new(pos.x, pos.y, final_size.x, final_size.y));
        }
        final_size
    }

    fn draw(&self, _widget: &Widget, _ctx: &mut DrawingContext) {}

    fn handle_routed_message(
        &mut self,
        _widget: &mut Widget,
        _msg: &mut UiMessage,
        _emit: &mut Vec<UiMessage>,
    ) {
    }
}
