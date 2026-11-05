//! What an object stands on, for the editor's Drop to Ground, Align to Surface and Find Floating Objects.
//!
//! Pure geometry: a triangle BVH per mesh (read back from the GPU once, shared by every entity drawing it), rays,
//! and the rules that turn hits into a drop or a verdict. The host gathers the surfaces and applies the answer as an
//! undoable transform.

use glam::{Mat4, Vec3};

/// Gaps up to this are contact: a drop does nothing.
pub const CONTACT: f32 = 0.03;
/// Only a gap over this is reported as floating (a leaf's edge a few centimetres up is not news).
pub const FLOAT_GAP: f32 = 0.05;
/// How far below an object the ground is looked for.
pub const SEARCH_DEPTH: f32 = 200.0;
/// Objects wider than this are structure (floors, walls, whole buildings), not props, and are never reported.
pub const PROP_SPAN: f32 = 12.0;

/// A ray hit: distance along the (unit) ray, and the surface's world normal facing the ray.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Hit {
    /// Metres along the ray.
    pub distance: f32,
    /// Unit world normal, facing back along the ray.
    pub normal: Vec3,
}

// ─── Triangle BVH ───────────────────────────────────────────────────────────

#[derive(Clone, Copy, Debug)]
struct Node {
    min: Vec3,
    max: Vec3,
    /// Leaf: first triangle. Inner: index of the left child (the right one follows it).
    first: u32,
    /// Leaf: triangle count. Inner: zero.
    count: u32,
}

/// A mesh's triangles in its own space, ordered into a bounding-volume hierarchy.
pub struct TriangleBvh {
    nodes: Vec<Node>,
    triangles: Vec<[Vec3; 3]>,
}

fn bounds(triangles: &[[Vec3; 3]]) -> (Vec3, Vec3) {
    let mut min = Vec3::splat(f32::INFINITY);
    let mut max = Vec3::splat(f32::NEG_INFINITY);
    for triangle in triangles {
        for corner in triangle {
            min = min.min(*corner);
            max = max.max(*corner);
        }
    }
    (min, max)
}

impl TriangleBvh {
    /// Build from indexed positions. Degenerate and out-of-range triangles are dropped.
    #[must_use]
    pub fn build(positions: &[Vec3], indices: &[u32]) -> Self {
        let mut triangles: Vec<[Vec3; 3]> = indices
            .chunks_exact(3)
            .filter_map(|t| {
                let tri = [
                    *positions.get(t[0] as usize)?,
                    *positions.get(t[1] as usize)?,
                    *positions.get(t[2] as usize)?,
                ];
                ((tri[1] - tri[0]).cross(tri[2] - tri[0]).length_squared() > 1e-14).then_some(tri)
            })
            .collect();
        let mut nodes = Vec::with_capacity(triangles.len() / 2 + 1);
        let (min, max) = bounds(&triangles);
        nodes.push(Node { min, max, first: 0, count: triangles.len() as u32 });
        let mut stack = vec![0usize];
        while let Some(at) = stack.pop() {
            let Node { min, max, first, count } = nodes[at];
            if count <= 4 {
                continue;
            }
            let extent = max - min;
            let axis = if extent.x >= extent.y && extent.x >= extent.z {
                0
            } else if extent.y >= extent.z {
                1
            } else {
                2
            };
            let slice = &mut triangles[first as usize..(first + count) as usize];
            let centre = |t: &[Vec3; 3]| (t[0][axis] + t[1][axis] + t[2][axis]) / 3.0;
            slice.sort_unstable_by(|a, b| centre(a).total_cmp(&centre(b)));
            let half = count / 2;
            let left = nodes.len() as u32;
            for (start, n) in [(first, half), (first + half, count - half)] {
                let (min, max) = bounds(&triangles[start as usize..(start + n) as usize]);
                nodes.push(Node { min, max, first: start, count: n });
            }
            nodes[at] = Node { min, max, first: left, count: 0 };
            stack.push(left as usize);
            stack.push(left as usize + 1);
        }
        Self { nodes, triangles }
    }

    /// Whether there are no triangles.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.triangles.is_empty()
    }

    /// Every triangle corner, in the mesh's own space.
    pub fn corners(&self) -> impl Iterator<Item = Vec3> + '_ {
        self.triangles.iter().flatten().copied()
    }

    /// The nearest hit of a ray in the mesh's own space within `max_t` (in units of `direction`), with the
    /// geometric normal (unnormalised, not flipped).
    #[must_use]
    pub fn raycast(&self, origin: Vec3, direction: Vec3, max_t: f32) -> Option<(f32, Vec3)> {
        if self.triangles.is_empty() {
            return None;
        }
        let inverse = direction.recip();
        let mut best: Option<(f32, Vec3)> = None;
        let mut stack = vec![0usize];
        while let Some(at) = stack.pop() {
            let node = self.nodes[at];
            let limit = best.map_or(max_t, |(t, _)| t);
            if !slab(origin, inverse, node.min, node.max, limit) {
                continue;
            }
            if node.count == 0 {
                stack.push(node.first as usize);
                stack.push(node.first as usize + 1);
                continue;
            }
            for triangle in &self.triangles[node.first as usize..(node.first + node.count) as usize] {
                if let Some(t) = intersect(origin, direction, triangle)
                    && t < best.map_or(max_t, |(b, _)| b)
                {
                    best = Some((t, (triangle[1] - triangle[0]).cross(triangle[2] - triangle[0])));
                }
            }
        }
        best
    }
}

/// Whether the ray meets the box before `limit`. Division by a zero component gives infinities, which the min/max
/// below order correctly.
fn slab(origin: Vec3, inverse: Vec3, min: Vec3, max: Vec3, limit: f32) -> bool {
    let a = (min - origin) * inverse;
    let b = (max - origin) * inverse;
    let near = a.min(b).max_element().max(0.0);
    let far = a.max(b).min_element().min(limit);
    !near.is_nan() && !far.is_nan() && near <= far
}

/// Möller–Trumbore, two-sided.
fn intersect(origin: Vec3, direction: Vec3, [a, b, c]: &[Vec3; 3]) -> Option<f32> {
    let (e1, e2) = (*b - *a, *c - *a);
    let p = direction.cross(e2);
    let det = e1.dot(p);
    if det.abs() < 1e-12 {
        return None;
    }
    let inv = 1.0 / det;
    let s = origin - *a;
    let u = s.dot(p) * inv;
    if !(0.0..=1.0).contains(&u) {
        return None;
    }
    let q = s.cross(e1);
    let v = direction.dot(q) * inv;
    if v < 0.0 || u + v > 1.0 {
        return None;
    }
    let t = e2.dot(q) * inv;
    (t > 0.0).then_some(t)
}

// ─── Surfaces in the world ──────────────────────────────────────────────────

/// One mesh drawn somewhere: its BVH and where it is.
pub struct MeshSurface<'a> {
    /// The mesh's triangles.
    pub bvh: &'a TriangleBvh,
    /// Its world matrix.
    pub model: Mat4,
    /// `model`'s inverse.
    pub inverse: Mat4,
    /// World bounds, for culling.
    pub min: Vec3,
    /// World bounds, for culling.
    pub max: Vec3,
}

impl MeshSurface<'_> {
    /// The nearest world hit along a unit `direction` within `max_distance`.
    #[must_use]
    pub fn raycast(&self, origin: Vec3, direction: Vec3, max_distance: f32) -> Option<Hit> {
        let end = origin + direction * max_distance;
        if end.min(origin).cmpgt(self.max).any() || end.max(origin).cmplt(self.min).any() {
            return None;
        }
        let local_origin = self.inverse.transform_point3(origin);
        let local_direction = self.inverse.transform_vector3(direction);
        // `t` is in units of the local direction, which is the world direction's length per world metre, so the
        // same `t` measures world metres along a unit world direction.
        let (t, normal) = self.bvh.raycast(local_origin, local_direction, max_distance)?;
        let mut normal = self.inverse.transpose().transform_vector3(normal).normalize_or_zero();
        if normal.dot(direction) > 0.0 {
            normal = -normal;
        }
        Some(Hit { distance: t, normal })
    }
}

/// Everything an object might stand on or touch.
pub struct Ground<'a> {
    /// Every mesh that might be underfoot.
    pub meshes: Vec<MeshSurface<'a>>,
    /// Terrain: the nearest hit along a unit ray, if any.
    pub terrain: Option<&'a dyn Fn(Vec3, Vec3, f32) -> Option<Hit>>,
}

impl Ground<'_> {
    /// The nearest hit along a unit `direction`.
    #[must_use]
    pub fn raycast(&self, origin: Vec3, direction: Vec3, max_distance: f32) -> Option<Hit> {
        let mut best = self.terrain.and_then(|terrain| terrain(origin, direction, max_distance));
        for mesh in &self.meshes {
            let limit = best.map_or(max_distance, |hit| hit.distance);
            if let Some(hit) = mesh.raycast(origin, direction, limit)
                && best.is_none_or(|b| hit.distance < b.distance)
            {
                best = Some(hit);
            }
        }
        best
    }
}

// ─── The rules ──────────────────────────────────────────────────────────────

/// Nine points across the bottom of a box, inset a little so a ray does not graze a neighbour's side.
#[must_use]
pub fn footprint(min: Vec3, max: Vec3) -> Vec<Vec3> {
    let inset = (max - min) * 0.1;
    let (lo, hi) = (min + inset, max - inset);
    let mut points = Vec::with_capacity(9);
    for fx in [0.0, 0.5, 1.0] {
        for fz in [0.0, 0.5, 1.0] {
            points.push(Vec3::new(lo.x + (hi.x - lo.x) * fx, min.y, lo.z + (hi.z - lo.z) * fz));
        }
    }
    points
}

/// Rays start this far above the object's lowest point, so a sunk object still finds the surface it is sunk in,
/// while a table top above a chair pushed under it is not mistaken for its ground.
#[must_use]
pub fn ray_lift(min: Vec3, max: Vec3) -> f32 {
    ((max.y - min.y) * 0.5).clamp(0.02, 0.5)
}

/// What is under an object whose world bounds are `min..max`.
#[derive(Clone, Debug, PartialEq)]
pub struct Support {
    /// Gap from the object's lowest point down to the highest surface under its footprint. Negative when sunk.
    pub gap: Option<f32>,
    /// The averaged normal of the surfaces under it.
    pub normal: Vec3,
    /// Something touches its top or a side (a lamp hung from a ceiling, a sign on a wall).
    pub touching: bool,
}

/// Up to `count` of `points`, spread out: each next one the farthest from those already taken (in XZ), starting
/// from the one nearest their middle. An object's base is sampled where it is, not where its box is.
#[must_use]
pub fn spread(points: &[Vec3], count: usize) -> Vec<Vec3> {
    if points.len() <= count {
        return points.to_vec();
    }
    let flat = |p: Vec3| glam::Vec2::new(p.x, p.z);
    let middle = points.iter().fold(glam::Vec2::ZERO, |sum, p| sum + flat(*p)) / points.len() as f32;
    let first = points
        .iter()
        .copied()
        .min_by(|a, b| flat(*a).distance_squared(middle).total_cmp(&flat(*b).distance_squared(middle)))
        .expect("more points than count");
    let mut taken = vec![first];
    let mut nearest: Vec<f32> = points.iter().map(|p| flat(*p).distance_squared(flat(first))).collect();
    while taken.len() < count {
        let (at, far) = nearest.iter().enumerate().fold((0, -1.0), |best, (i, d)| if *d > best.1 { (i, *d) } else { best });
        if far <= 1e-6 {
            break;
        }
        let next = points[at];
        taken.push(next);
        for (i, p) in points.iter().enumerate() {
            nearest[i] = nearest[i].min(flat(*p).distance_squared(flat(next)));
        }
    }
    taken
}

/// The lowest of an object's world-space vertices: within a few centimetres of its bottom, spread over its base.
#[must_use]
pub fn base_points(vertices: &[Vec3], count: usize) -> Vec<Vec3> {
    let Some(bottom) = vertices.iter().map(|v| v.y).min_by(f32::total_cmp) else {
        return Vec::new();
    };
    let top = vertices.iter().map(|v| v.y).max_by(f32::total_cmp).unwrap_or(bottom);
    let band = (0.03 * (top - bottom)).clamp(0.02, 0.1);
    let low: Vec<Vec3> = vertices.iter().copied().filter(|v| v.y <= bottom + band).collect();
    spread(&low, count)
}

/// Look under, over and beside an object whose bounds are `min..max`, from its box's footprint. `ground` must leave
/// out the object itself.
#[must_use]
pub fn support(ground: &Ground<'_>, min: Vec3, max: Vec3) -> Support {
    support_at(ground, &footprint(min, max), min, max)
}

/// [`support`] from given base points (an object's own lowest vertices): a tree stands on its trunk, not on the box
/// round its crown.
#[must_use]
pub fn support_at(ground: &Ground<'_>, base: &[Vec3], min: Vec3, max: Vec3) -> Support {
    let lift = ray_lift(min, max);
    let mut gap: Option<f32> = None;
    let mut normal = Vec3::ZERO;
    for &point in base {
        let start = point + Vec3::Y * lift;
        if let Some(hit) = ground.raycast(start, Vec3::NEG_Y, SEARCH_DEPTH + lift) {
            let below = hit.distance - lift;
            gap = Some(gap.map_or(below, |g: f32| g.min(below)));
            normal += hit.normal;
        }
    }
    let centre = (min + max) * 0.5;
    let reach = CONTACT + 0.02;
    let touching = [
        (Vec3::new(centre.x, max.y - 0.01, centre.z), Vec3::Y),
        (Vec3::new(min.x + 0.01, centre.y, centre.z), Vec3::NEG_X),
        (Vec3::new(max.x - 0.01, centre.y, centre.z), Vec3::X),
        (Vec3::new(centre.x, centre.y, min.z + 0.01), Vec3::NEG_Z),
        (Vec3::new(centre.x, centre.y, max.z - 0.01), Vec3::Z),
    ]
    .into_iter()
    .any(|(from, direction)| ground.raycast(from, direction, reach).is_some());
    Support { gap, normal: normal.normalize_or(Vec3::Y), touching }
}

/// How far to move an object down (negative: up) to rest on what is under it, or `None` when nothing is.
#[must_use]
pub fn drop_offset(support: &Support) -> Option<f32> {
    support.gap.filter(|gap| gap.abs() > 0.001)
}

/// Whether an object floats: something is under it, it touches nothing, and the gap is more than contact.
#[must_use]
pub fn floats(support: &Support, min: Vec3, max: Vec3) -> bool {
    let span = (max.x - min.x).max(max.z - min.z);
    span <= PROP_SPAN && !support.touching && support.gap.is_some_and(|gap| gap > FLOAT_GAP)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A unit quad facing up at height 0, `size` metres across.
    fn floor(size: f32) -> (Vec<Vec3>, Vec<u32>) {
        let h = size * 0.5;
        (
            vec![Vec3::new(-h, 0.0, -h), Vec3::new(h, 0.0, -h), Vec3::new(h, 0.0, h), Vec3::new(-h, 0.0, h)],
            vec![0, 2, 1, 0, 3, 2],
        )
    }

    fn surface(bvh: &TriangleBvh, model: Mat4, min: Vec3, max: Vec3) -> MeshSurface<'_> {
        MeshSurface { bvh, model, inverse: model.inverse(), min, max }
    }

    #[test]
    fn a_bvh_finds_the_nearest_of_many_triangles() {
        // A 20 x 20 grid of floor tiles, one of them raised.
        let mut positions = Vec::new();
        let mut indices = Vec::new();
        for i in 0..20 {
            for j in 0..20 {
                let y = if (i, j) == (7, 11) { 0.5 } else { 0.0 };
                let base = positions.len() as u32;
                let (x, z) = (i as f32, j as f32);
                positions.extend([
                    Vec3::new(x, y, z),
                    Vec3::new(x + 1.0, y, z),
                    Vec3::new(x + 1.0, y, z + 1.0),
                    Vec3::new(x, y, z + 1.0),
                ]);
                indices.extend([base, base + 2, base + 1, base, base + 3, base + 2]);
            }
        }
        let bvh = TriangleBvh::build(&positions, &indices);
        let (t, _) = bvh.raycast(Vec3::new(7.5, 10.0, 11.5), Vec3::NEG_Y, 100.0).unwrap();
        assert!((t - 9.5).abs() < 1e-4);
        let (t, _) = bvh.raycast(Vec3::new(3.5, 10.0, 3.5), Vec3::NEG_Y, 100.0).unwrap();
        assert!((t - 10.0).abs() < 1e-4);
        assert!(bvh.raycast(Vec3::new(30.0, 10.0, 3.5), Vec3::NEG_Y, 100.0).is_none());
    }

    #[test]
    fn a_scaled_and_moved_mesh_answers_in_world_metres() {
        let (positions, indices) = floor(1.0);
        let bvh = TriangleBvh::build(&positions, &indices);
        let model = Mat4::from_scale_rotation_translation(Vec3::new(10.0, 3.0, 10.0), glam::Quat::IDENTITY, Vec3::Y * 2.0);
        let mesh = surface(&bvh, model, Vec3::new(-5.0, 2.0, -5.0), Vec3::new(5.0, 2.0, 5.0));
        let hit = mesh.raycast(Vec3::new(1.0, 7.0, 1.0), Vec3::NEG_Y, 100.0).unwrap();
        assert!((hit.distance - 5.0).abs() < 1e-4, "{}", hit.distance);
        assert!(hit.normal.dot(Vec3::Y) > 0.99, "the normal faces back up the ray");
    }

    #[test]
    fn a_crate_floating_over_a_floor_drops_by_its_gap() {
        let (positions, indices) = floor(1.0);
        let bvh = TriangleBvh::build(&positions, &indices);
        let model = Mat4::from_scale(Vec3::new(50.0, 1.0, 50.0));
        let ground = Ground {
            meshes: vec![surface(&bvh, model, Vec3::new(-25.0, 0.0, -25.0), Vec3::new(25.0, 0.0, 25.0))],
            terrain: None,
        };
        let (min, max) = (Vec3::new(-0.5, 0.3, -0.5), Vec3::new(0.5, 1.3, 0.5));
        let support = support(&ground, min, max);
        assert!((drop_offset(&support).unwrap() - 0.3).abs() < 1e-4);
        assert!(floats(&support, min, max));

        // Resting on it: no drop, not floating.
        let (min, max) = (Vec3::new(-0.5, 0.0, -0.5), Vec3::new(0.5, 1.0, 0.5));
        let support = super::support(&ground, min, max);
        assert!(drop_offset(&support).is_none());
        assert!(!floats(&support, min, max));
    }

    #[test]
    fn a_sunk_object_comes_up_to_the_surface() {
        let (positions, indices) = floor(1.0);
        let bvh = TriangleBvh::build(&positions, &indices);
        let ground = Ground {
            meshes: vec![surface(&bvh, Mat4::from_scale(Vec3::splat(50.0)), Vec3::splat(-25.0), Vec3::splat(25.0))],
            terrain: None,
        };
        let support = support(&ground, Vec3::new(-0.5, -0.2, -0.5), Vec3::new(0.5, 0.8, 0.5));
        assert!((drop_offset(&support).unwrap() + 0.2).abs() < 1e-4, "negative: move up");
    }

    #[test]
    fn on_a_slope_the_highest_ground_under_it_stops_the_drop() {
        let terrain = |origin: Vec3, direction: Vec3, max: f32| {
            // The plane y = 0.1 x, hit from above.
            if direction != Vec3::NEG_Y {
                return None;
            }
            let ground = 0.1 * origin.x;
            let t = origin.y - ground;
            (t >= 0.0 && t <= max).then(|| Hit { distance: t, normal: Vec3::new(-0.1, 1.0, 0.0).normalize() })
        };
        let ground = Ground { meshes: Vec::new(), terrain: Some(&terrain) };
        // A 2 m wide box from x = -1 to 1, bottom at 1.0: the ground under its right edge is at 0.08 (inset).
        let support = support(&ground, Vec3::new(-1.0, 1.0, -1.0), Vec3::new(1.0, 2.0, 1.0));
        assert!((drop_offset(&support).unwrap() - 0.92).abs() < 1e-3);
        assert!(support.normal.x < 0.0, "the normal leans with the slope");
    }

    #[test]
    fn a_lamp_hung_from_a_ceiling_does_not_float() {
        let (positions, indices) = floor(1.0);
        let bvh = TriangleBvh::build(&positions, &indices);
        let floor_model = Mat4::from_scale(Vec3::new(20.0, 1.0, 20.0));
        let ceiling_model = Mat4::from_scale_rotation_translation(Vec3::new(20.0, 1.0, 20.0), glam::Quat::IDENTITY, Vec3::Y * 3.0);
        let ground = Ground {
            meshes: vec![
                surface(&bvh, floor_model, Vec3::new(-10.0, 0.0, -10.0), Vec3::new(10.0, 0.0, 10.0)),
                surface(&bvh, ceiling_model, Vec3::new(-10.0, 3.0, -10.0), Vec3::new(10.0, 3.0, 10.0)),
            ],
            terrain: None,
        };
        let (min, max) = (Vec3::new(-0.2, 2.4, -0.2), Vec3::new(0.2, 3.0, 0.2));
        let support = support(&ground, min, max);
        assert!(support.touching);
        assert!(!floats(&support, min, max));
    }

    #[test]
    fn a_tree_stands_on_its_trunk_not_on_the_box_round_its_crown() {
        // A 0.6 m wall top at x = 0..0.6, 2 m up; a fig whose trunk stands on it and whose crown spreads 4 m east.
        let (positions, indices) = floor(1.0);
        let bvh = TriangleBvh::build(&positions, &indices);
        let wall_top = Mat4::from_scale_rotation_translation(Vec3::new(0.6, 1.0, 0.6), glam::Quat::IDENTITY, Vec3::new(0.3, 2.0, 0.0));
        let ground_model = Mat4::from_scale(Vec3::new(40.0, 1.0, 40.0));
        let ground = Ground {
            meshes: vec![
                surface(&bvh, wall_top, Vec3::new(0.0, 2.0, -0.3), Vec3::new(0.6, 2.0, 0.3)),
                surface(&bvh, ground_model, Vec3::new(-20.0, 0.0, -20.0), Vec3::new(20.0, 0.0, 20.0)),
            ],
            terrain: None,
        };
        let trunk = [Vec3::new(0.25, 2.0, 0.0), Vec3::new(0.35, 2.0, 0.05), Vec3::new(0.3, 2.0, -0.05)];
        let crown = [Vec3::new(4.5, 3.5, 0.0), Vec3::new(0.2, 4.0, 1.0), Vec3::new(4.4, 3.0, -1.0)];
        let vertices: Vec<Vec3> = trunk.iter().chain(&crown).copied().collect();
        let (min, max) = (Vec3::new(0.2, 2.0, -1.0), Vec3::new(4.5, 4.0, 1.0));
        assert!(floats(&support(&ground, min, max), min, max), "the box's footprint misses the wall: the old test");
        let base = base_points(&vertices, 9);
        assert_eq!(base.len(), 3, "only the trunk's vertices are at the bottom");
        let support = support_at(&ground, &base, min, max);
        assert!(!floats(&support, min, max), "standing on its trunk, it does not float");
        assert!(drop_offset(&support).is_none());
    }

    #[test]
    fn spread_points_cover_the_base_rather_than_clustering() {
        let points: Vec<Vec3> = (0..100).map(|i| Vec3::new((i % 10) as f32, 0.0, (i / 10) as f32)).collect();
        let picked = spread(&points, 4);
        assert_eq!(picked.len(), 4);
        let xs: Vec<f32> = picked.iter().map(|p| p.x).collect();
        assert!(xs.iter().any(|x| *x <= 1.0) && xs.iter().any(|x| *x >= 8.0), "{xs:?}: both ends are sampled");
    }

    #[test]
    fn structure_wider_than_a_prop_is_never_reported() {
        let support = Support { gap: Some(3.0), normal: Vec3::Y, touching: false };
        assert!(!floats(&support, Vec3::new(0.0, 3.0, 0.0), Vec3::new(30.0, 3.2, 30.0)));
        assert!(floats(&support, Vec3::new(0.0, 3.0, 0.0), Vec3::new(1.0, 3.2, 1.0)));
    }
}
