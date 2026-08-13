// Port of: example_repo/fyrox/Fyrox-master/fyrox-ui/src/widget.rs
// Stripped of Fyrox-specific: InheritableVariable, StyleResource, ImmutableString,
// Matrix3 transforms, drag-drop, tooltips, context menus, Material, Reflect/Visit.
// Only the layout + rendering essentials remain.

use crate::{
    message::NodeHandle,
    types::{HorizontalAlignment, Rect, Thickness, VerticalAlignment},
};
use glam::Vec2;

/// Core layout/appearance data for every widget in the hierarchy.
/// Analogous to Fyrox's `Widget` struct but without reflection machinery.
#[derive(Debug)]
pub struct Widget {
    pub handle: NodeHandle,
    pub name: String,

    // --- explicit size constraints ---
    /// NaN = let layout decide
    pub width: f32,
    pub height: f32,
    pub min_size: Vec2,
    pub max_size: Vec2,

    // --- layout hints ---
    pub desired_local_position: Vec2,
    pub horizontal_alignment: HorizontalAlignment,
    pub vertical_alignment: VerticalAlignment,
    pub margin: Thickness,
    pub row: usize,
    pub column: usize,

    // --- visual ---
    pub background: [u8; 4],
    pub foreground: [u8; 4],
    pub visibility: bool,
    pub enabled: bool,
    pub clip_to_bounds: bool,
    pub hit_test_visibility: bool,
    pub z_index: usize,
    /// Hover label shown by the shell after `theme::TOOLTIP_DELAY_MS`.
    pub tooltip: String,

    // --- hierarchy ---
    pub parent: NodeHandle,
    pub children: Vec<NodeHandle>,

    // --- layout result cache (computed by UserInterface::perform_layout) ---
    pub measure_valid: bool,
    pub arrange_valid: bool,
    pub prev_measure: Vec2,
    pub prev_arrange: Rect,
    pub desired_size: Vec2,
    pub actual_local_position: Vec2,
    pub actual_local_size: Vec2,
    pub clip_bounds: Rect,
    pub global_visibility: bool,
}

impl Default for Widget {
    fn default() -> Self {
        Self {
            handle: NodeHandle::NONE,
            name: String::new(),
            width: f32::NAN,
            height: f32::NAN,
            min_size: Vec2::ZERO,
            max_size: Vec2::splat(f32::INFINITY),
            desired_local_position: Vec2::ZERO,
            horizontal_alignment: HorizontalAlignment::default(),
            vertical_alignment: VerticalAlignment::default(),
            margin: Thickness::ZERO,
            row: 0,
            column: 0,
            background: [50, 50, 50, 255],
            foreground: [220, 220, 220, 255],
            visibility: true,
            enabled: true,
            clip_to_bounds: true,
            hit_test_visibility: true,
            z_index: 0,
            tooltip: String::new(),
            parent: NodeHandle::NONE,
            children: Vec::new(),
            measure_valid: false,
            arrange_valid: false,
            prev_measure: Vec2::ZERO,
            prev_arrange: Rect::ZERO,
            desired_size: Vec2::ZERO,
            actual_local_position: Vec2::ZERO,
            actual_local_size: Vec2::ZERO,
            clip_bounds: Rect::ZERO,
            global_visibility: true,
        }
    }
}

impl Widget {
    pub fn actual_size(&self) -> Vec2 {
        self.actual_local_size
    }

    pub fn screen_bounds(&self) -> Rect {
        Rect::from_pos_size(self.actual_local_position, self.actual_local_size)
    }

    pub fn invalidate_layout(&mut self) {
        self.measure_valid = false;
        self.arrange_valid = false;
    }
}

/// Builder for Widget — call before wrapping in a concrete control type.
pub struct WidgetBuilder {
    pub widget: Widget,
}

impl WidgetBuilder {
    pub fn new() -> Self {
        Self {
            widget: Widget::default(),
        }
    }

    pub fn with_name(mut self, name: impl Into<String>) -> Self {
        self.widget.name = name.into();
        self
    }
    pub fn with_width(mut self, w: f32) -> Self {
        self.widget.width = w;
        self
    }
    pub fn with_height(mut self, h: f32) -> Self {
        self.widget.height = h;
        self
    }
    pub fn with_min_size(mut self, s: Vec2) -> Self {
        self.widget.min_size = s;
        self
    }
    pub fn with_max_size(mut self, s: Vec2) -> Self {
        self.widget.max_size = s;
        self
    }
    pub fn with_margin(mut self, m: Thickness) -> Self {
        self.widget.margin = m;
        self
    }
    pub fn with_horizontal_alignment(mut self, a: HorizontalAlignment) -> Self {
        self.widget.horizontal_alignment = a;
        self
    }
    pub fn with_vertical_alignment(mut self, a: VerticalAlignment) -> Self {
        self.widget.vertical_alignment = a;
        self
    }
    pub fn with_background(mut self, color: [u8; 4]) -> Self {
        self.widget.background = color;
        self
    }
    pub fn with_foreground(mut self, color: [u8; 4]) -> Self {
        self.widget.foreground = color;
        self
    }
    pub fn with_visibility(mut self, v: bool) -> Self {
        self.widget.visibility = v;
        self
    }
    pub fn with_enabled(mut self, e: bool) -> Self {
        self.widget.enabled = e;
        self
    }
    pub fn with_row(mut self, r: usize) -> Self {
        self.widget.row = r;
        self
    }
    pub fn with_column(mut self, c: usize) -> Self {
        self.widget.column = c;
        self
    }
    pub fn with_children(mut self, ch: impl IntoIterator<Item = NodeHandle>) -> Self {
        self.widget.children.extend(ch);
        self
    }
    pub fn with_desired_position(mut self, p: Vec2) -> Self {
        self.widget.desired_local_position = p;
        self
    }

    pub fn with_tooltip(mut self, t: impl Into<String>) -> Self {
        self.widget.tooltip = t.into();
        self
    }

    pub fn with_hit_test_visibility(mut self, v: bool) -> Self {
        self.widget.hit_test_visibility = v;
        self
    }

    pub fn with_clip_to_bounds(mut self, v: bool) -> Self {
        self.widget.clip_to_bounds = v;
        self
    }

    /// Consume the builder and return the finished Widget.
    pub fn build(self) -> Widget {
        self.widget
    }
}

impl Default for WidgetBuilder {
    fn default() -> Self {
        Self::new()
    }
}
