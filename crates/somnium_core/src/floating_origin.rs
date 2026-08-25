//! CPU-side integer-grid plus local-float world coordinates.

use glam::Vec3;

/// Large-world position with exact integer cell and small local offset.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GlobalPosition {
    /// Exact grid coordinate.
    pub cell: [i64; 3],
    /// Local metres kept near zero for f32 precision.
    pub offset: Vec3,
}

impl GlobalPosition {
    /// Canonicalize an authored f64 position into grid plus local offset.
    #[must_use]
    pub fn from_world(position: [f64; 3], cell_size: f64) -> Self {
        assert!(cell_size.is_finite() && cell_size > 0.0);
        let cell = position.map(|value| (value / cell_size).floor() as i64);
        let offset = Vec3::new(
            (position[0] - cell[0] as f64 * cell_size) as f32,
            (position[1] - cell[1] as f64 * cell_size) as f32,
            (position[2] - cell[2] as f64 * cell_size) as f32,
        );
        Self { cell, offset }
    }

    /// Camera-relative f32 position without forming a huge f32 world value.
    #[must_use]
    pub fn relative_to(self, origin: Self, cell_size: f64) -> Vec3 {
        let delta = Vec3::new(
            (self.cell[0] - origin.cell[0]) as f32,
            (self.cell[1] - origin.cell[1]) as f32,
            (self.cell[2] - origin.cell[2]) as f32,
        );
        delta * cell_size as f32 + (self.offset - origin.offset)
    }
}

/// Floating-origin owner. Rebase changes the render origin, not authored data.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FloatingOrigin {
    /// Current camera-aligned origin.
    pub origin: GlobalPosition,
    /// Grid cell size in metres.
    pub cell_size: f64,
}

impl FloatingOrigin {
    /// Start at a world position.
    #[must_use]
    pub fn new(world: [f64; 3], cell_size: f64) -> Self {
        let mut origin = GlobalPosition::from_world(world, cell_size);
        origin.offset = Vec3::ZERO;
        Self { origin, cell_size }
    }

    /// Rebase when the camera enters another integer cell.
    pub fn update(&mut self, camera: [f64; 3]) -> bool {
        let mut next = GlobalPosition::from_world(camera, self.cell_size);
        next.offset = Vec3::ZERO;
        if next.cell == self.origin.cell {
            return false;
        }
        self.origin = next;
        true
    }

    /// Render-relative position.
    #[must_use]
    pub fn relative(self, position: GlobalPosition) -> Vec3 {
        position.relative_to(self.origin, self.cell_size)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kilometre_scale_neighbours_keep_centimetre_differences() {
        let origin = GlobalPosition::from_world([10_000_000.0, 0.0, 10_000_000.0], 256.0);
        let point = GlobalPosition::from_world([10_000_000.01, 0.0, 10_000_000.02], 256.0);
        let relative = point.relative_to(origin, 256.0);
        assert!((relative.x - 0.01).abs() < 0.001);
        assert!((relative.z - 0.02).abs() < 0.001);
    }

    #[test]
    fn rebasing_changes_only_the_origin_cell() {
        let point = GlobalPosition::from_world([300.0, 2.0, 3.0], 256.0);
        let mut origin = FloatingOrigin::new([0.0; 3], 256.0);
        let before = origin.relative(point);
        assert!(origin.update([300.0, 0.0, 0.0]));
        let after = origin.relative(point);
        assert!((before.x - after.x - 256.0).abs() < f32::EPSILON);
        assert_eq!(point, GlobalPosition::from_world([300.0, 2.0, 3.0], 256.0));
    }
}
