// Common geometry/layout types for Somnium native UI.
// Port of: example_repo/fyrox/Fyrox-master/fyrox-ui/src/{alignment.rs, thickness.rs}

use glam::Vec2;

#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub enum HorizontalAlignment {
    #[default]
    Stretch,
    Left,
    Center,
    Right,
}

#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub enum VerticalAlignment {
    #[default]
    Stretch,
    Top,
    Center,
    Bottom,
}

/// Axis-uniform margin / padding — left, right, top, bottom (all in logical pixels).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Thickness {
    pub left:   f32,
    pub right:  f32,
    pub top:    f32,
    pub bottom: f32,
}

impl Thickness {
    pub const ZERO: Self = Self { left: 0.0, right: 0.0, top: 0.0, bottom: 0.0 };

    pub fn uniform(v: f32) -> Self {
        Self { left: v, right: v, top: v, bottom: v }
    }

    pub fn axes(h: f32, v: f32) -> Self {
        Self { left: h, right: h, top: v, bottom: v }
    }

    /// Total horizontal extent (left + right).
    pub fn h(&self) -> f32 { self.left + self.right }
    /// Total vertical extent (top + bottom).
    pub fn v(&self) -> f32 { self.top + self.bottom }
    /// Top-left corner offset as Vec2.
    pub fn offset(&self) -> Vec2 { Vec2::new(self.left, self.top) }
}

impl Default for Thickness {
    fn default() -> Self { Self::ZERO }
}

/// Axis-aligned rectangle in logical pixels (position + size).
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

impl Rect {
    pub const ZERO: Self = Self { x: 0.0, y: 0.0, w: 0.0, h: 0.0 };
    pub const INF: Self = Self { x: 0.0, y: 0.0, w: f32::INFINITY, h: f32::INFINITY };

    pub fn new(x: f32, y: f32, w: f32, h: f32) -> Self { Self { x, y, w, h } }
    pub fn from_pos_size(pos: Vec2, size: Vec2) -> Self {
        Self { x: pos.x, y: pos.y, w: size.x, h: size.y }
    }
    pub fn pos(&self) -> Vec2 { Vec2::new(self.x, self.y) }
    pub fn size(&self) -> Vec2 { Vec2::new(self.w, self.h) }

    pub fn contains(&self, p: Vec2) -> bool {
        p.x >= self.x && p.x < self.x + self.w &&
        p.y >= self.y && p.y < self.y + self.h
    }

    pub fn intersect(&self, other: &Self) -> Self {
        let x = self.x.max(other.x);
        let y = self.y.max(other.y);
        let r = (self.x + self.w).min(other.x + other.w);
        let b = (self.y + self.h).min(other.y + other.h);
        Self { x, y, w: (r - x).max(0.0), h: (b - y).max(0.0) }
    }
}
