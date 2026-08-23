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
    message::{MessageDirection, Modifiers, NodeHandle, UiMessage, WidgetMessage},
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

/// Ownership record for the one modal-feeling gesture currently in flight.
/// Gesture-specific restoration data stays in the owning control.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GestureToken {
    pub owner: NodeHandle,
}

#[derive(Debug, Clone, Copy)]
struct ModalFocus {
    root: NodeHandle,
    return_to: NodeHandle,
}

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
    /// Device pixels per unit of layout space.
    ///
    /// The widget tree lays out in **logical units**, so every density token
    /// (`theme::TITLEBAR_HEIGHT`, row heights, the 68 px pre-scene budget) means
    /// the same apparent size at every DPI. This factor exists only at the two
    /// boundaries where the OS speaks device pixels: pointer positions coming
    /// in, and the scissor rect going out.
    ///
    /// Before Phase 27 the tree was fed `window.inner_size()` directly, which
    /// winit reports in physical pixels, so at 200 % the whole chrome rendered
    /// at half its intended apparent size.
    ui_scale: f32,
    focused_ih: IH,
    #[allow(dead_code)]
    captured_ih: IH,
    hovered_ih: IH,
    modifiers: Modifiers,
    active_gesture: Option<GestureToken>,
    modal_focus: Option<ModalFocus>,
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
            ui_scale: 1.0,
            focused_ih: IH::NONE,
            captured_ih: IH::NONE,
            hovered_ih: IH::NONE,
            modifiers: Modifiers::default(),
            active_gesture: None,
            modal_focus: None,
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

    /// True if `handle` is `ancestor` or a descendant of it.
    pub fn is_under(&self, handle: NodeHandle, ancestor: NodeHandle) -> bool {
        if handle.is_none() || ancestor.is_none() {
            return false;
        }
        let mut h = to_ih(handle);
        for _ in 0..64 {
            if to_nh(h) == ancestor {
                return true;
            }
            let parent = match self.nodes.try_borrow(h) {
                Ok(n) => to_ih(n.widget.parent),
                Err(_) => return false,
            };
            if parent.is_none() {
                return false;
            }
            h = parent;
        }
        false
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
        if let Some(modal) = self.modal_focus {
            if handle.is_none() || !self.is_under(handle, modal.root) {
                return;
            }
        }
        self.focused_ih = to_ih(handle);
        self.bring_focus_into_view(handle);
    }

    /// Enter a modal focus scope and remember the invoking control. Attempts
    /// to focus outside `root` are ignored until the matching modal exits.
    pub fn enter_modal(&mut self, root: NodeHandle, initial_focus: NodeHandle) {
        let return_to = self.focused();
        self.modal_focus = Some(ModalFocus { root, return_to });
        self.focused_ih = to_ih(initial_focus);
        self.bring_focus_into_view(initial_focus);
    }

    /// Leave a modal focus scope and return focus to the control which opened
    /// it, if that control still exists and is visible.
    pub fn exit_modal(&mut self, root: NodeHandle) -> NodeHandle {
        let Some(modal) = self.modal_focus.filter(|m| m.root == root) else {
            return self.focused();
        };
        self.modal_focus = None;
        let target = if modal.return_to.is_some()
            && self.nodes.try_borrow(to_ih(modal.return_to)).is_ok()
            && self.is_globally_visible(modal.return_to)
        {
            modal.return_to
        } else {
            NodeHandle::NONE
        };
        self.focused_ih = to_ih(target);
        self.bring_focus_into_view(target);
        target
    }

    pub fn modal_root(&self) -> Option<NodeHandle> {
        self.modal_focus.map(|m| m.root)
    }

    /// Current modifiers at the OS boundary. Programmatic input can use this
    /// to preserve the same self-contained message contract.
    pub fn modifiers(&self) -> Modifiers {
        self.modifiers
    }

    pub fn set_modifiers(&mut self, modifiers: Modifiers) {
        self.modifiers = modifiers;
    }

    fn keyboard_message(&self, code: winit::keyboard::KeyCode, pressed: bool) -> WidgetMessage {
        if pressed {
            WidgetMessage::KeyDown(code, self.modifiers)
        } else {
            WidgetMessage::KeyUp(code, self.modifiers)
        }
    }

    pub fn active_gesture(&self) -> Option<GestureToken> {
        self.active_gesture
    }

    /// Cancel the active gesture before any overlay is dismissed. Returns true
    /// only when a control accepted cancellation.
    pub fn cancel_active_gesture(&mut self) -> bool {
        let Some(token) = self.active_gesture.take() else {
            return false;
        };
        let mut emit = Vec::new();
        let cancelled = if let Ok(node) = self.nodes.try_borrow_mut(to_ih(token.owner)) {
            let (widget, control) = (&mut node.widget, &mut node.control);
            control.cancel_gesture(widget, &mut emit)
        } else {
            false
        };
        for message in emit {
            self.message_queue.push_back(message);
        }
        if cancelled {
            self.captured_ih = IH::NONE;
            self.invalidate_ancestors(token.owner);
        }
        cancelled
    }

    /// Scroll one target node into a specific ScrollViewer.
    pub fn scroll_into_view(&mut self, viewer: NodeHandle, target: NodeHandle) -> bool {
        let target_bounds = self.focus_bounds_of(target);
        self.scroll_rect_into_view(viewer, target_bounds)
    }

    fn focus_bounds_of(&self, handle: NodeHandle) -> Rect {
        self.nodes
            .try_borrow(to_ih(handle))
            .map(|n| n.control.focus_bounds(&n.widget))
            .unwrap_or(Rect::ZERO)
    }

    fn scroll_rect_into_view(&mut self, viewer: NodeHandle, target: Rect) -> bool {
        let changed = if let Ok(node) = self.nodes.try_borrow_mut(to_ih(viewer)) {
            let (widget, control) = (&mut node.widget, &mut node.control);
            control.scroll_into_view(widget, target)
        } else {
            false
        };
        if changed {
            self.invalidate_ancestors(viewer);
        }
        changed
    }

    /// Bring a node (or a composite control's focused row) into every ancestor
    /// scroll viewport.
    pub fn bring_focus_into_view(&mut self, handle: NodeHandle) {
        if handle.is_none() {
            return;
        }
        let target = self.focus_bounds_of(handle);
        let mut ancestor = self.parent_of(handle);
        while let Some(parent) = ancestor {
            self.scroll_rect_into_view(parent, target);
            ancestor = self.parent_of(parent);
        }
    }

    fn collect_focusable(&self, handle: NodeHandle, out: &mut Vec<NodeHandle>) {
        let (focusable, children) = {
            let Ok(node) = self.nodes.try_borrow(to_ih(handle)) else {
                return;
            };
            if !node.widget.global_visibility || !node.widget.enabled {
                return;
            }
            (
                node.control.is_keyboard_focusable(),
                node.widget.children.clone(),
            )
        };
        if focusable {
            out.push(handle);
        }
        for child in children {
            self.collect_focusable(child, out);
        }
    }

    /// Arrow traversal for a region whose rows are ordinary child controls.
    /// TreeView handles the same keys internally because its rows are virtual.
    pub fn traverse_region(&mut self, region: NodeHandle, key: winit::keyboard::KeyCode) -> bool {
        use winit::keyboard::KeyCode;
        let mut stops = Vec::new();
        self.collect_focusable(region, &mut stops);
        if stops.is_empty() {
            return false;
        }
        let current = self.focused();
        let current_index = stops.iter().position(|h| *h == current);
        let next = match key {
            KeyCode::ArrowDown => current_index.map_or(0, |i| (i + 1).min(stops.len() - 1)),
            KeyCode::ArrowUp => current_index.map_or(stops.len() - 1, |i| i.saturating_sub(1)),
            KeyCode::Home => 0,
            KeyCode::End => stops.len() - 1,
            _ => return false,
        };
        let target = stops[next];
        if current != target {
            if current.is_some() {
                self.send(UiMessage::new(
                    current,
                    MessageDirection::ToWidget,
                    WidgetMessage::Unfocus,
                ));
            }
            self.focused_ih = to_ih(target);
            self.send(UiMessage::new(
                target,
                MessageDirection::ToWidget,
                WidgetMessage::Focus,
            ));
        }
        self.bring_focus_into_view(target);
        true
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
                        // Widget.invalidate_layout only dirties this node. Ancestors
                        // still hold cached measure and would skip the child.
                        let dirty = self
                            .nodes
                            .try_borrow(current_ih)
                            .map(|n| !n.widget.measure_valid)
                            .unwrap_or(false);
                        if dirty {
                            self.invalidate_ancestors(to_nh(current_ih));
                        }
                        let gesture_active = self
                            .nodes
                            .try_borrow(current_ih)
                            .map(|n| n.control.gesture_active())
                            .unwrap_or(false);
                        let owner = to_nh(current_ih);
                        if gesture_active && self.active_gesture.is_none() {
                            self.active_gesture = Some(GestureToken { owner });
                        } else if !gesture_active
                            && self
                                .active_gesture
                                .is_some_and(|token| token.owner == owner)
                        {
                            self.active_gesture = None;
                        }
                        if msg.handled
                            && matches!(
                                msg.data::<WidgetMessage>(),
                                Some(WidgetMessage::KeyDown(..))
                            )
                        {
                            self.bring_focus_into_view(msg.destination);
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
        // Overlay pass, still inside this node's clip: whatever a container
        // needs to paint on top of its own content.
        {
            let node = self.nodes.borrow_mut(handle);
            let widget_ptr = &node.widget as *const Widget;
            let control_ptr = node.control.as_ref() as *const dyn Control;
            // SAFETY: as above — the widget lives in the pool record and
            // `draw_ctx` is a separate allocation, so the two borrows cannot
            // alias.
            unsafe {
                (*control_ptr).draw_over(&*widget_ptr, &mut self.draw_ctx);
            }
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
    /// Whether a node and every ancestor is visible. Used by keyboard
    /// traversal so Tab cannot land on a hidden region.
    pub fn is_globally_visible(&self, handle: NodeHandle) -> bool {
        self.nodes
            .try_borrow(handle.transmute())
            .map(|n| n.widget.global_visibility)
            .unwrap_or(false)
    }

    /// Parent of a node, or `None` if the handle is stale or is the root.
    pub fn parent_of(&self, handle: NodeHandle) -> Option<NodeHandle> {
        let parent = self
            .nodes
            .try_borrow(handle.transmute())
            .ok()?
            .widget
            .parent;
        (!parent.is_none()).then_some(parent)
    }

    /// Current value of a numeric control, or `None` for any other widget.
    pub fn numeric_value_of(&self, handle: NodeHandle) -> Option<f32> {
        self.nodes
            .try_borrow(handle.transmute())
            .ok()?
            .control
            .numeric_value()
    }

    pub fn add_font(&mut self, bytes: &[u8]) -> Result<u8, &'static str> {
        self.draw_ctx.font_atlas.add_font(bytes)
    }

    // -----------------------------------------------------------------------
    // Resize
    // -----------------------------------------------------------------------

    /// Device pixels per layout unit. Feed `Window::scale_factor()`.
    pub fn set_ui_scale(&mut self, scale: f32) {
        self.ui_scale = scale.clamp(0.5, 8.0);
    }

    pub fn ui_scale(&self) -> f32 {
        self.ui_scale
    }

    /// Convert an OS pointer position (device pixels) into layout units.
    pub fn to_logical(&self, physical_x: f64, physical_y: f64) -> Vec2 {
        Vec2::new(
            physical_x as f32 / self.ui_scale,
            physical_y as f32 / self.ui_scale,
        )
    }

    /// Resize the tree. `w` and `h` are **logical units**, not device pixels.
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
            WindowEvent::ModifiersChanged(modifiers) => {
                let state = modifiers.state();
                self.modifiers = Modifiers {
                    ctrl: state.control_key(),
                    shift: state.shift_key(),
                    alt: state.alt_key(),
                    logo: state.super_key(),
                };
                false
            }
            WindowEvent::CursorMoved { position, .. } => {
                self.cursor_pos = self.to_logical(position.x, position.y);
                let captured = to_nh(self.captured_ih);
                if captured.is_some() {
                    let pos = self.cursor_pos;
                    self.send(UiMessage::new(
                        captured,
                        MessageDirection::ToWidget,
                        WidgetMessage::MouseMove {
                            pos,
                            mods: self.modifiers,
                        },
                    ));
                }
                let hit = self.hit_test(self.cursor_pos);
                let over_viewport = !hit.is_some() || hit == self.viewport_handle;
                let new_hover = if over_viewport { IH::NONE } else { to_ih(hit) };
                if new_hover != self.hovered_ih {
                    if self.hovered_ih.is_some() {
                        self.send(UiMessage::new(
                            to_nh(self.hovered_ih),
                            MessageDirection::ToWidget,
                            WidgetMessage::MouseLeave,
                        ));
                    }
                    if new_hover.is_some() {
                        self.send(UiMessage::new(
                            to_nh(new_hover),
                            MessageDirection::ToWidget,
                            WidgetMessage::MouseEnter,
                        ));
                    }
                    self.hovered_ih = new_hover;
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
                    if self
                        .modal_focus
                        .is_some_and(|modal| !self.is_under(hit, modal.root))
                    {
                        return true;
                    }
                    let old = to_nh(self.focused_ih);
                    if old != hit && old.is_some() {
                        self.send(UiMessage::new(
                            old,
                            MessageDirection::ToWidget,
                            WidgetMessage::Unfocus,
                        ));
                    }
                    self.set_focus(hit);
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
                                mods: self.modifiers,
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
                                mods: self.modifiers,
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
                        WidgetMessage::MouseWheel {
                            pos,
                            delta: d,
                            mods: self.modifiers,
                        },
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
                        let wmsg =
                            self.keyboard_message(code, key_ev.state == ElementState::Pressed);
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

#[cfg(test)]
mod overlay_order_tests {
    use super::*;
    use crate::widget::WidgetBuilder;
    use crate::widgets::{
        border::BorderBuilder, scroll_viewer::ScrollViewerBuilder, stack_panel::StackPanelBuilder,
    };

    /// A container that paints in `draw()` and a marker that paints in
    /// `draw_over()`, so the ordering can be asserted without depending on any
    /// particular widget's internals.
    struct OrderProbe;

    impl Control for OrderProbe {
        fn draw(&self, widget: &Widget, ctx: &mut DrawingContext) {
            ctx.push_rect_filled(widget.screen_bounds(), [1, 1, 1, 255]);
        }
        fn draw_over(&self, widget: &Widget, ctx: &mut DrawingContext) {
            ctx.push_rect_filled(widget.screen_bounds(), [3, 3, 3, 255]);
        }
    }

    #[test]
    fn draw_over_paints_after_every_child() {
        // The defect this hook exists for: a container's overlay emitted from
        // `draw()` lands *under* its children, so it renders as nothing at all.
        let mut ui = UserInterface::new(200.0, 200.0);
        let root = ui.root();

        let probe = UiNode::new(
            WidgetBuilder::new()
                .with_width(100.0)
                .with_height(100.0)
                .build(),
            Box::new(OrderProbe),
        );
        let probe_h = ui.add_node(probe, root);

        let child = BorderBuilder::new(
            WidgetBuilder::new()
                .with_width(50.0)
                .with_height(50.0)
                .with_background([2, 2, 2, 255]),
        )
        .with_stroke_thickness(crate::types::Thickness::ZERO)
        .build();
        ui.add_node(child, probe_h);

        ui.perform_layout();
        ui.draw();

        let order: Vec<u8> = ui
            .draw_ctx
            .instances
            .iter()
            .map(|p| p.fill_a[0])
            .filter(|c| (1..=3).contains(c))
            .collect();

        let under = order.iter().position(|c| *c == 1).expect("draw() ran");
        let content = order.iter().position(|c| *c == 2).expect("child ran");
        let over = order.iter().position(|c| *c == 3).expect("draw_over ran");

        assert!(under < content, "draw() must paint beneath its children");
        assert!(
            content < over,
            "draw_over() must paint above its children, got {order:?}"
        );
    }

    #[test]
    fn a_scroll_viewer_paints_its_bar_above_the_content() {
        // Regression guard for the real bug: the scrollbar and the edge fades
        // were emitted from `draw()` and were therefore covered by whatever was
        // scrolling underneath them.
        let mut ui = UserInterface::new(300.0, 120.0);
        let root = ui.root();

        let sv =
            ScrollViewerBuilder::new(WidgetBuilder::new().with_width(300.0).with_height(120.0))
                .build();
        let sv_h = ui.add_node(sv, root);

        // Content taller than the viewport, so the bar is live.
        let column = StackPanelBuilder::new(
            WidgetBuilder::new()
                .with_width(280.0)
                .with_height(600.0)
                .with_background([2, 2, 2, 255]),
        )
        .build();
        ui.add_node(column, sv_h);

        ui.perform_layout();
        ui.draw();

        let instances = &ui.draw_ctx.instances;
        let last_content = instances
            .iter()
            .rposition(|p| p.fill_a == [2, 2, 2, 255])
            .expect("the scrolled content must paint");

        // The bar track uses the input surface; find it after the content.
        let bar = crate::theme::active().semantic.surface.input.bytes();
        let bar_after = instances.iter().skip(last_content).any(|p| p.fill_a == bar);
        assert!(
            bar_after,
            "the scrollbar must paint after the content it scrolls"
        );
    }
}

#[cfg(test)]
mod input_contract_tests {
    use super::*;
    use crate::message::{MessageDirection, UiMessage};
    use crate::widget::WidgetBuilder;
    use crate::widgets::{
        check_box::{CheckBoxBuilder, CheckBoxMessage},
        numeric_field::{NumericFieldBuilder, NumericFieldMessage},
        popup::{PopupBuilder, PopupMessage},
        scroll_viewer::ScrollViewerBuilder,
        slider::{SliderBuilder, SliderMessage},
        stack_panel::StackPanelBuilder,
        tree_view::{TreeItem, TreeViewBuilder, TreeViewMessage},
    };
    use glam::Vec2;
    use std::sync::{Arc, Mutex};

    struct ModifierProbe(Arc<Mutex<Option<Modifiers>>>);

    impl Control for ModifierProbe {
        fn handle_routed_message(
            &mut self,
            _widget: &mut Widget,
            msg: &mut UiMessage,
            _emit: &mut Vec<UiMessage>,
        ) {
            if let Some(WidgetMessage::KeyDown(_, modifiers)) = msg.data::<WidgetMessage>() {
                *self.0.lock().expect("probe mutex") = Some(*modifiers);
                msg.handled = true;
            }
        }
    }

    fn bounds_of(ui: &UserInterface, handle: NodeHandle) -> crate::types::Rect {
        ui.nodes
            .try_borrow(handle.transmute())
            .expect("handle stays valid")
            .widget
            .screen_bounds()
    }

    fn top_of(ui: &UserInterface, handle: NodeHandle) -> f32 {
        ui.nodes
            .try_borrow(handle.transmute())
            .expect("handle stays valid")
            .widget
            .screen_bounds()
            .y
    }

    /// A scroll viewer whose content is taller than its viewport, laid out.
    /// Returns (viewer, content).
    fn scrollable(ui: &mut UserInterface) -> (NodeHandle, NodeHandle) {
        let root = ui.root();
        let sv =
            ScrollViewerBuilder::new(WidgetBuilder::new().with_width(300.0).with_height(120.0))
                .build();
        let sv_h = ui.add_node(sv, root);
        let column =
            StackPanelBuilder::new(WidgetBuilder::new().with_width(280.0).with_height(600.0))
                .build();
        let col_h = ui.add_node(column, sv_h);
        ui.perform_layout();
        (sv_h, col_h)
    }

    fn wheel(ui: &mut UserInterface, target: NodeHandle, delta: f32) -> Vec<UiMessage> {
        ui.send(UiMessage::new(
            target,
            MessageDirection::ToWidget,
            WidgetMessage::MouseWheel {
                pos: Vec2::new(100.0, 50.0),
                delta,
                mods: Modifiers::default(),
            },
        ));
        let out = ui.update();
        ui.perform_layout();
        out
    }

    #[test]
    fn shift_held_is_delivered_to_the_focused_widget() {
        let mut ui = UserInterface::new(200.0, 80.0);
        let observed = Arc::new(Mutex::new(None));
        let probe = UiNode::new(
            WidgetBuilder::new()
                .with_width(100.0)
                .with_height(24.0)
                .build(),
            Box::new(ModifierProbe(observed.clone())),
        );
        let handle = ui.add_node(probe, ui.root());
        ui.set_focus(handle);
        ui.set_modifiers(Modifiers {
            shift: true,
            ..Modifiers::default()
        });
        let key = ui.keyboard_message(crate::message::KeyCode::KeyA, true);
        ui.send(UiMessage::new(handle, MessageDirection::ToWidget, key));
        ui.update();

        let delivered = observed
            .lock()
            .expect("probe mutex")
            .expect("key delivered");
        assert!(delivered.shift);
        assert!(!delivered.ctrl && !delivered.alt && !delivered.logo);
    }

    #[test]
    fn arrow_traversal_in_a_long_tree_scrolls_the_focused_row_into_view() {
        let mut ui = UserInterface::new(300.0, 120.0);
        let viewer = ui.add_node(
            ScrollViewerBuilder::new(WidgetBuilder::new().with_width(300.0).with_height(120.0))
                .build(),
            ui.root(),
        );
        let tree = ui.add_node(
            TreeViewBuilder::new(WidgetBuilder::new().with_width(280.0)).build(),
            viewer,
        );
        let items: Vec<TreeItem> = (0..30)
            .map(|id| TreeItem {
                id,
                label: format!("Row {id}"),
                depth: 0,
                icon: crate::icons::IconId::EmptyEntity,
                has_children: false,
                expanded: false,
            })
            .collect();
        ui.send(TreeViewMessage::set_items(tree, items));
        ui.update();
        ui.perform_layout();
        ui.set_focus(tree);
        ui.send(UiMessage::new(
            tree,
            MessageDirection::ToWidget,
            WidgetMessage::KeyDown(crate::message::KeyCode::End, Modifiers::default()),
        ));
        let outgoing = ui.update();
        ui.perform_layout();

        assert!(outgoing.iter().any(|message| matches!(
            message.data::<TreeViewMessage>(),
            Some(TreeViewMessage::Select(29))
        )));
        let viewport = bounds_of(&ui, viewer);
        let tree_bounds = bounds_of(&ui, tree);
        let focused_bottom = tree_bounds.y + 30.0 * crate::theme::TREE_ROW_HEIGHT;
        assert!(
            tree_bounds.y < viewport.y,
            "the long tree must have scrolled"
        );
        assert!(
            focused_bottom <= viewport.y + viewport.h + 0.5,
            "focused row bottom {focused_bottom} must be inside viewport {:?}",
            viewport
        );
    }

    #[test]
    fn cancelling_a_scrub_restores_the_value_without_closing_an_open_popup() {
        let mut ui = UserInterface::new(320.0, 120.0);
        let root = ui.root();
        let field = ui.add_node(
            NumericFieldBuilder::new(WidgetBuilder::new().with_width(200.0).with_height(24.0))
                .with_value(10.0)
                .with_drag_step(1.0)
                .build(),
            root,
        );
        let popup = ui.add_node(PopupBuilder::new(WidgetBuilder::new()).build(), root);
        ui.send(UiMessage::new(
            popup,
            MessageDirection::ToWidget,
            PopupMessage::Open,
        ));
        ui.update();
        ui.perform_layout();
        let bounds = bounds_of(&ui, field);
        let start = Vec2::new(bounds.x + bounds.w - 8.0, bounds.y + bounds.h * 0.5);
        ui.send(UiMessage::new(
            field,
            MessageDirection::ToWidget,
            WidgetMessage::MouseDown {
                pos: start,
                button: crate::message::MouseButton::Left,
                mods: Modifiers::default(),
            },
        ));
        ui.update();
        ui.send(UiMessage::new(
            field,
            MessageDirection::ToWidget,
            WidgetMessage::MouseMove {
                pos: start + Vec2::new(20.0, 0.0),
                mods: Modifiers::default(),
            },
        ));
        ui.update();
        assert_ne!(ui.numeric_value_of(field), Some(10.0));
        assert_eq!(ui.active_gesture(), Some(GestureToken { owner: field }));

        assert!(
            ui.cancel_active_gesture(),
            "Esc precedence must find the scrub"
        );
        let outgoing = ui.update();
        assert_eq!(ui.numeric_value_of(field), Some(10.0));
        assert!(
            ui.nodes
                .try_borrow(popup.transmute())
                .expect("popup exists")
                .widget
                .visibility,
            "cancelling the gesture must not dismiss the popup below it"
        );
        assert!(outgoing.iter().any(|message| matches!(
            message.data::<NumericFieldMessage>(),
            Some(NumericFieldMessage::ValueChanging(value)) if *value == 10.0
        )));
    }

    #[test]
    fn modal_focus_is_trapped_and_returns_to_its_invoker() {
        let mut ui = UserInterface::new(300.0, 120.0);
        let root = ui.root();
        let invoker = ui.add_node(
            UiNode::new(WidgetBuilder::new().build(), Box::new(RootControl)),
            root,
        );
        let modal = ui.add_node(
            UiNode::new(WidgetBuilder::new().build(), Box::new(RootControl)),
            root,
        );
        let inside = ui.add_node(
            UiNode::new(WidgetBuilder::new().build(), Box::new(RootControl)),
            modal,
        );
        ui.set_focus(invoker);
        ui.enter_modal(modal, inside);
        ui.set_focus(invoker);
        assert_eq!(ui.focused(), inside, "focus cannot escape an active modal");
        assert_eq!(ui.exit_modal(modal), invoker);
        assert_eq!(ui.focused(), invoker);
    }

    #[test]
    fn a_short_sibling_does_not_make_tall_content_unscrollable() {
        // The reported Details bug. The panel stacks the property list against
        // an empty state inside one scroll viewer; `arrange_override` assigned
        // `content_h` per child instead of accumulating, so the short trailing
        // sibling won and the whole panel reported itself as viewport-sized.
        // Symptom: no scrolling and no visible thumb, on every entity with more
        // properties than fit.
        let mut ui = UserInterface::new(400.0, 200.0);
        let root = ui.root();
        let sv =
            ScrollViewerBuilder::new(WidgetBuilder::new().with_width(300.0).with_height(120.0))
                .build();
        let sv_h = ui.add_node(sv, root);

        let tall =
            StackPanelBuilder::new(WidgetBuilder::new().with_width(280.0).with_height(600.0))
                .build();
        let tall_h = ui.add_node(tall, sv_h);

        // Added *after* the tall child, exactly as the Details empty state is.
        let short =
            StackPanelBuilder::new(WidgetBuilder::new().with_width(280.0).with_height(90.0))
                .build();
        ui.add_node(short, sv_h);

        ui.perform_layout();
        let before = top_of(&ui, tall_h);
        wheel(&mut ui, sv_h, -60.0);
        assert!(
            top_of(&ui, tall_h) < before,
            "a tall child must still scroll when a short sibling follows it"
        );
    }

    #[test]
    fn a_hidden_child_reserves_no_scroll_height() {
        // With the property stack hidden, Details should not pretend to have
        // 600 px of content it is not showing.
        let mut ui = UserInterface::new(400.0, 200.0);
        let root = ui.root();
        let sv =
            ScrollViewerBuilder::new(WidgetBuilder::new().with_width(300.0).with_height(120.0))
                .build();
        let sv_h = ui.add_node(sv, root);

        let tall =
            StackPanelBuilder::new(WidgetBuilder::new().with_width(280.0).with_height(600.0))
                .build();
        let tall_h = ui.add_node(tall, sv_h);
        ui.perform_layout();

        ui.set_visibility(tall_h, false);
        ui.perform_layout();
        ui.draw();

        // Nothing visible is taller than the viewport, so the thumb must render
        // in its inactive colour rather than claiming the panel scrolls.
        let inactive = crate::theme::active().semantic.border.default.bytes();
        let claims_scrollable = ui
            .draw_ctx
            .instances
            .iter()
            .any(|p| p.fill_a == crate::theme::active().semantic.border.strong.bytes());
        assert!(
            !claims_scrollable,
            "a viewer whose only tall child is hidden must not report as scrollable"
        );
        let _ = inactive;
    }

    #[test]
    fn the_mouse_wheel_scrolls_the_content() {
        // The regression this module exists for: ScrollViewer's whole
        // `handle_routed_message` was deleted and all 184 tests still passed,
        // because none of them scrolled anything.
        let mut ui = UserInterface::new(400.0, 200.0);
        let (sv, content) = scrollable(&mut ui);
        let before = top_of(&ui, content);
        wheel(&mut ui, sv, -40.0);
        let after = top_of(&ui, content);
        assert!(
            after < before,
            "wheel down must move the content up: {before} -> {after}"
        );
    }

    #[test]
    fn scrolling_is_clamped_at_both_ends() {
        let mut ui = UserInterface::new(400.0, 200.0);
        let (sv, content) = scrollable(&mut ui);

        wheel(&mut ui, sv, 100_000.0);
        let at_top = top_of(&ui, content);
        wheel(&mut ui, sv, 100_000.0);
        assert_eq!(
            top_of(&ui, content),
            at_top,
            "must not scroll above the top"
        );

        wheel(&mut ui, sv, -100_000.0);
        let at_bottom = top_of(&ui, content);
        wheel(&mut ui, sv, -100_000.0);
        assert_eq!(
            top_of(&ui, content),
            at_bottom,
            "must not scroll past the end"
        );
        assert!(
            at_bottom < at_top,
            "the content must actually be scrollable"
        );
    }

    #[test]
    fn clicking_the_scroll_track_jumps_the_view() {
        let mut ui = UserInterface::new(400.0, 200.0);
        let (sv, content) = scrollable(&mut ui);
        let before = top_of(&ui, content);
        // Derived from the live bounds: the root centres its child, so a
        // hard-coded point lands outside the 10 px gutter and the click is
        // simply ignored.
        let b = bounds_of(&ui, sv);
        let point = Vec2::new(b.x + b.w - 4.0, b.y + b.h * 0.9);
        ui.send(UiMessage::new(
            sv,
            MessageDirection::ToWidget,
            WidgetMessage::MouseDown {
                pos: point,
                button: crate::message::MouseButton::Left,
                mods: Modifiers::default(),
            },
        ));
        ui.update();
        ui.perform_layout();
        assert!(
            top_of(&ui, content) < before,
            "a track click must move the view"
        );
    }

    #[test]
    fn a_checkbox_reports_its_new_state_when_set() {
        let mut ui = UserInterface::new(200.0, 60.0);
        let root = ui.root();
        let cb =
            CheckBoxBuilder::new(WidgetBuilder::new().with_width(160.0).with_height(24.0)).build();
        let cb_h = ui.add_node(cb, root);
        ui.perform_layout();

        ui.send(CheckBoxMessage::set_checked(cb_h, true));
        ui.update();
        ui.perform_layout();
        ui.draw();

        // A ticked box paints the Check glyph; an unticked one does not.
        let (uv, _) =
            crate::icons::IconId::Check.draw_quad(crate::types::Rect::new(0.0, 0.0, 16.0, 16.0));
        let tick_u0 = uv[0].x;
        let ticked = ui
            .draw_ctx
            .instances
            .iter()
            .any(|p| (p.uv[0] - tick_u0).abs() < 1e-6);
        assert!(ticked, "a checked box must paint its tick");
    }

    #[test]
    fn a_tree_view_hovers_the_row_under_the_pointer() {
        // The Outliner hover animation reads this. If it stopped being set the
        // rows would simply never highlight, and nothing else would notice.
        let mut ui = UserInterface::new(300.0, 200.0);
        let root = ui.root();
        let tv =
            TreeViewBuilder::new(WidgetBuilder::new().with_width(280.0).with_height(180.0)).build();
        let tv_h = ui.add_node(tv, root);
        ui.perform_layout();

        let item = |id: u32, label: &str| TreeItem {
            id,
            label: label.into(),
            depth: 0,
            has_children: false,
            expanded: false,
            icon: crate::icons::IconId::Camera,
        };
        ui.send(TreeViewMessage::set_items(
            tv_h,
            vec![item(1, "Camera"), item(2, "Terrain")],
        ));
        ui.update();
        ui.perform_layout();

        let hover = crate::style::tree_row(crate::style::VisualState::with(
            crate::style::Interaction::Hover,
        ));
        assert_ne!(hover.background[3], 0, "the hover recipe must be visible");

        let painted = |ui: &UserInterface| {
            ui.draw_ctx
                .instances
                .iter()
                .any(|p| p.fill_a == hover.background)
        };

        ui.draw();
        assert!(!painted(&ui), "nothing is hovered before the pointer moves");

        ui.send(UiMessage::new(
            tv_h,
            MessageDirection::ToWidget,
            WidgetMessage::MouseMove {
                pos: {
                    let b = bounds_of(&ui, tv_h);
                    Vec2::new(b.x + 40.0, b.y + crate::theme::TREE_ROW_HEIGHT * 1.5)
                },
                mods: Modifiers::default(),
            },
        ));
        ui.update();
        ui.perform_layout();
        ui.draw();
        assert!(
            painted(&ui),
            "the row under the pointer must paint a hover fill"
        );
    }

    #[test]
    fn pressing_a_slider_track_emits_a_value_change() {
        let mut ui = UserInterface::new(200.0, 40.0);
        let root = ui.root();
        let sl =
            SliderBuilder::new(WidgetBuilder::new().with_width(160.0).with_height(20.0)).build();
        let sl_h = ui.add_node(sl, root);
        ui.perform_layout();

        let b = bounds_of(&ui, sl_h);
        ui.send(UiMessage::new(
            sl_h,
            MessageDirection::ToWidget,
            WidgetMessage::MouseDown {
                pos: Vec2::new(b.x + b.w * 0.75, b.y + b.h * 0.5),
                button: crate::message::MouseButton::Left,
                mods: Modifiers::default(),
            },
        ));
        let emitted = ui.update();
        assert!(
            emitted
                .iter()
                .any(|m| matches!(m.data::<SliderMessage>(), Some(SliderMessage::Value(_)))),
            "pressing the track must emit a value change"
        );
    }
}
