//! Painted foliage binned by ground cell, so a frame visits only the cells within reach of the camera instead of
//! every instance of every terrain (Rimeborn's WS1-6: a dressed 4 x 4 km tile runs to a million instances). Also keeps
//! the sorted list of kinds painted, which the frame used to rebuild by sorting every instance.
//!
//! Rebuilt only when the terrain's foliage changes: its edit revision (every brush stroke and load bumps it), the
//! instance count, and the first and last instances must all match the last build.

use somnium_renderer::terrain::foliage_paint::PaintedFoliage;

/// Ground metres per cell. Small enough that a 60 m grass reach visits a handful of cells, large enough that a
/// 400 m tree reach is a few hundred.
const CELL: f32 = 32.0;

#[derive(Default)]
pub(crate) struct FoliageGrid {
    key: Option<(u64, usize, [u32; 8])>,
    min: [f32; 2],
    dims: [i32; 2],
    /// Instance indices ordered by cell; cell `c`'s are `order[starts[c]..starts[c + 1]]`.
    order: Vec<u32>,
    starts: Vec<u32>,
    /// The kinds painted, sorted, each once.
    pub kinds: Vec<u8>,
}

fn ends(instances: &[PaintedFoliage]) -> [u32; 8] {
    let bits = |p: Option<&PaintedFoliage>| {
        p.map_or([0; 4], |p| [p.position.x.to_bits(), p.position.y.to_bits(), p.position.z.to_bits(), u32::from(p.kind)])
    };
    let (a, b) = (bits(instances.first()), bits(instances.last()));
    [a[0], a[1], a[2], a[3], b[0], b[1], b[2], b[3]]
}

impl FoliageGrid {
    /// Bring the grid up to date with `instances` (terrain-local positions).
    pub fn refresh(&mut self, revision: u64, instances: &[PaintedFoliage]) {
        let key = (revision, instances.len(), ends(instances));
        if self.key == Some(key) {
            return;
        }
        self.key = Some(key);
        let mut kinds: Vec<u8> = instances.iter().map(|p| p.kind).collect();
        kinds.sort_unstable();
        kinds.dedup();
        self.kinds = kinds;
        if instances.is_empty() {
            (self.order, self.starts, self.dims) = (Vec::new(), vec![0], [0, 0]);
            return;
        }
        let (mut lo, mut hi) = ([f32::MAX; 2], [f32::MIN; 2]);
        for p in instances {
            lo = [lo[0].min(p.position.x), lo[1].min(p.position.z)];
            hi = [hi[0].max(p.position.x), hi[1].max(p.position.z)];
        }
        self.min = lo;
        self.dims = [((hi[0] - lo[0]) / CELL) as i32 + 1, ((hi[1] - lo[1]) / CELL) as i32 + 1];
        let cells = (self.dims[0] * self.dims[1]) as usize;
        let cell_of = |p: &PaintedFoliage| {
            let cx = ((p.position.x - lo[0]) / CELL) as i32;
            let cz = ((p.position.z - lo[1]) / CELL) as i32;
            (cz * self.dims[0] + cx) as usize
        };
        // Counting sort by cell: stable, so instances keep their painted order within a cell.
        let mut starts = vec![0u32; cells + 1];
        for p in instances {
            starts[cell_of(p) + 1] += 1;
        }
        for c in 0..cells {
            starts[c + 1] += starts[c];
        }
        let mut next = starts.clone();
        let mut order = vec![0u32; instances.len()];
        for (i, p) in instances.iter().enumerate() {
            let c = cell_of(p);
            order[next[c] as usize] = i as u32;
            next[c] += 1;
        }
        (self.order, self.starts) = (order, starts);
    }

    /// Every instance in the cells overlapping the square of half-side `reach` round `centre` (terrain-local x, z);
    /// an infinite reach is every instance. Callers still test the exact distance.
    pub fn visit(&self, centre: [f32; 2], reach: f32, mut f: impl FnMut(usize)) {
        if self.order.is_empty() {
            return;
        }
        if !reach.is_finite() {
            self.order.iter().for_each(|&i| f(i as usize));
            return;
        }
        // The square misses the grid entirely.
        let max = [self.min[0] + self.dims[0] as f32 * CELL, self.min[1] + self.dims[1] as f32 * CELL];
        if centre[0] + reach < self.min[0] || centre[1] + reach < self.min[1] || centre[0] - reach > max[0] || centre[1] - reach > max[1] {
            return;
        }
        let cell = |v: f32, lo: f32, n: i32| (((v - lo) / CELL).floor() as i32).clamp(0, n - 1);
        let (x0, x1) = (cell(centre[0] - reach, self.min[0], self.dims[0]), cell(centre[0] + reach, self.min[0], self.dims[0]));
        let (z0, z1) = (cell(centre[1] - reach, self.min[1], self.dims[1]), cell(centre[1] + reach, self.min[1], self.dims[1]));
        for cz in z0..=z1 {
            let row = (cz * self.dims[0]) as usize;
            let (a, b) = (self.starts[row + x0 as usize] as usize, self.starts[row + x1 as usize + 1] as usize);
            self.order[a..b].iter().for_each(|&i| f(i as usize));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use glam::Vec3;

    fn field() -> Vec<PaintedFoliage> {
        let mut seed = 7u32;
        let mut rand = move || {
            seed ^= seed << 13;
            seed ^= seed >> 17;
            seed ^= seed << 5;
            (seed >> 8) as f32 / (1 << 24) as f32
        };
        (0..5000)
            .map(|i| PaintedFoliage {
                kind: (128 + i % 7) as u8,
                position: Vec3::new(rand() * 1500.0 - 300.0, 20.0, rand() * 900.0 + 40.0),
                yaw: 0.0,
                tilt: 0.0,
                scale: 1.0,
            })
            .collect()
    }

    #[test]
    fn a_frame_visits_everything_in_reach_and_nothing_far() {
        let instances = field();
        let mut grid = FoliageGrid::default();
        grid.refresh(1, &instances);
        assert_eq!(grid.kinds, (128..135).collect::<Vec<u8>>());
        for (centre, reach) in [([200.0, 400.0], 60.0), ([-290.0, 50.0], 320.0), ([5000.0, 5000.0], 50.0), ([0.0, 0.0], 2000.0)] {
            let mut seen = vec![false; instances.len()];
            grid.visit(centre, reach, |i| seen[i] = true);
            for (i, p) in instances.iter().enumerate() {
                let (dx, dz) = (p.position.x - centre[0], p.position.z - centre[1]);
                if dx * dx + dz * dz <= reach * reach {
                    assert!(seen[i], "missed an instance in reach at {centre:?}");
                }
                if dx.abs() > reach + CELL || dz.abs() > reach + CELL {
                    assert!(!seen[i], "visited one far outside reach at {centre:?}");
                }
            }
        }
        let mut all = 0;
        grid.visit([0.0, 0.0], f32::INFINITY, |_| all += 1);
        assert_eq!(all, instances.len());
    }

    #[test]
    fn the_grid_rebuilds_when_the_foliage_changes() {
        let mut instances = field();
        let mut grid = FoliageGrid::default();
        grid.refresh(1, &instances);
        instances.push(PaintedFoliage { kind: 200, position: Vec3::new(10.0, 0.0, 10.0), yaw: 0.0, tilt: 0.0, scale: 1.0 });
        grid.refresh(1, &instances);
        assert!(grid.kinds.contains(&200), "a new instance is seen even at the same revision");
        let mut seen = false;
        grid.visit([10.0, 10.0], 5.0, |i| seen |= i == instances.len() - 1);
        assert!(seen);
    }
}
