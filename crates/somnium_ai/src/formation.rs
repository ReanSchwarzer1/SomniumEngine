//! Formations: a line abreast and a row behind it (spears in front, bows and
//! casters behind), laid out from an anchor and a facing. The group's
//! gameplay decides who stands where and when the line breaks; this keeps
//! the geometry: places, an order that crosses nobody over, and an anchor
//! that walks up to a holding distance from the enemy.
use glam::Vec3;

/// A line abreast at the anchor and a row behind it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Formation {
    /// The middle of the line.
    pub anchor: Vec3,
    /// Planar unit direction the line faces.
    pub facing: Vec3,
    /// Metres between neighbours in the line.
    pub line_gap: f32,
    /// Metres between neighbours in the row.
    pub row_gap: f32,
    /// How far behind the line the row stands.
    pub row_depth: f32,
}

impl Formation {
    /// A formation at `anchor` facing `facing` (made planar; +Z when it has
    /// no planar part), 1.6 m apart in the line, 2 m in a row 4 m behind.
    pub fn new(anchor: Vec3, facing: Vec3) -> Self {
        Self {
            anchor,
            facing: planar(facing, Vec3::Z),
            line_gap: 1.6,
            row_gap: 2.0,
            row_depth: 4.0,
        }
    }

    /// The line's left-to-right direction.
    pub fn right(&self) -> Vec3 {
        Vec3::new(-self.facing.z, 0.0, self.facing.x)
    }

    /// Places for `line` members abreast at the anchor and `row` members
    /// behind it, each left to right.
    pub fn slots(&self, line: usize, row: usize) -> (Vec<Vec3>, Vec<Vec3>) {
        let right = self.right();
        let spread = |n: usize, at: Vec3, gap: f32| {
            let middle = (n as f32 - 1.0) * 0.5;
            (0..n)
                .map(|i| at + right * (i as f32 - middle) * gap)
                .collect::<Vec<_>>()
        };
        (
            spread(line, self.anchor, self.line_gap),
            spread(
                row,
                self.anchor - self.facing * self.row_depth,
                self.row_gap,
            ),
        )
    }

    /// Indices of `positions` from left to right as they stand: give the
    /// k-th of them the k-th slot and nobody crosses another to its place.
    /// Order once, when the group forms; re-ordering as they move would swap
    /// neighbours who pass each other.
    pub fn left_to_right(&self, positions: &[Vec3]) -> Vec<usize> {
        let right = self.right();
        let mut order: Vec<usize> = (0..positions.len()).collect();
        order.sort_by(|a, b| {
            let side = |i: usize| (positions[i] - self.anchor).dot(right);
            side(*a).total_cmp(&side(*b))
        });
        order
    }

    /// Face `enemy` and walk the anchor at most `step` metres toward it,
    /// stopping `hold` metres short (on the ground plane).
    pub fn advance(&mut self, enemy: Vec3, hold: f32, step: f32) {
        let to = Vec3::new(enemy.x - self.anchor.x, 0.0, enemy.z - self.anchor.z);
        self.facing = planar(to, self.facing);
        let room = (to.length() - hold).max(0.0);
        self.anchor += self.facing * step.max(0.0).min(room);
    }
}

fn planar(v: Vec3, fallback: Vec3) -> Vec3 {
    Vec3::new(v.x, 0.0, v.z).try_normalize().unwrap_or(fallback)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_line_abreast_and_a_row_behind_in_standing_order() {
        let f = Formation::new(Vec3::ZERO, Vec3::Z);
        let (line, row) = f.slots(3, 2);
        // Abreast across the facing, 1.6 m apart, centred on the anchor.
        assert!(line.iter().all(|p| p.z.abs() < 1e-5));
        assert!((line[1] - line[0]).length() > 1.59 && line[1].length() < 1e-5);
        // The row 4 m behind, 2 m apart.
        assert!(row.iter().all(|p| (p.z + 4.0).abs() < 1e-5));
        assert!(((row[1] - row[0]).length() - 2.0).abs() < 1e-5);
        // Three standing about: ordered left to right along the line.
        let right = f.right();
        let at = [
            Vec3::new(3.0, 0.0, 1.0),
            Vec3::new(-3.0, 0.0, -1.0),
            Vec3::new(0.2, 0.0, 0.5),
        ];
        let order = f.left_to_right(&at);
        let sides: Vec<f32> = order.iter().map(|&i| at[i].dot(right)).collect();
        assert!(sides.windows(2).all(|w| w[0] <= w[1]), "{order:?}");
        // The slots run the same way, so nobody crosses.
        assert!(line[0].dot(right) < line[2].dot(right));
    }

    #[test]
    fn the_anchor_walks_up_and_holds_short_of_the_enemy() {
        let mut f = Formation::new(Vec3::ZERO, Vec3::X);
        let enemy = Vec3::new(0.0, 0.0, 20.0);
        f.advance(enemy, 9.0, 1.0);
        assert!((f.facing - Vec3::Z).length() < 1e-5, "turned to face it");
        assert!((f.anchor.z - 1.0).abs() < 1e-5);
        for _ in 0..100 {
            f.advance(enemy, 9.0, 1.0);
        }
        assert!(
            (f.anchor.z - 11.0).abs() < 1e-4,
            "held 9 m short: {}",
            f.anchor
        );
    }
}
