//! Private nearest-triangle query for the existing unsigned mesh-SDF bake.
//!
//! The tree changes only which distance tests are necessary, not the selected
//! triangle sample or the leaf distance calculation. It is discarded after one
//! brick is baked; GPU allocations and meshlet construction are unaffected.

use super::{Vertex, point_triangle_distance};
use glam::Vec3;

const LEAF_TRIANGLES: usize = 8;

#[derive(Clone, Copy)]
struct Triangle {
    a: Vec3,
    b: Vec3,
    c: Vec3,
}

impl Triangle {
    fn min(self) -> Vec3 {
        self.a.min(self.b).min(self.c)
    }

    fn max(self) -> Vec3 {
        self.a.max(self.b).max(self.c)
    }

    fn distance(self, p: Vec3) -> f32 {
        point_triangle_distance(p, self.a, self.b, self.c)
    }
}

struct Node {
    min: Vec3,
    max: Vec3,
    triangles: std::ops::Range<usize>,
    children: Option<(usize, usize)>,
}

impl Node {
    fn lower_bound_squared(&self, p: Vec3) -> f64 {
        // Expanded bounds and f64 arithmetic keep roundoff from rejecting a
        // candidate that the existing f32 triangle calculation could improve.
        // This allowance affects pruning only; returned distances stay unchanged.
        let padding = self.min.abs().max(self.max.abs()).max_element() as f64
            * (32.0 * f32::EPSILON as f64)
            + 1e-6;
        (0..3)
            .map(|axis| {
                let p = p[axis] as f64;
                let d = (self.min[axis] as f64 - padding - p)
                    .max(p - self.max[axis] as f64 - padding)
                    .max(0.0);
                d * d
            })
            .sum()
    }
}

pub(super) struct TriangleQuery {
    triangles: Vec<Triangle>,
    // Non-finite coordinates cannot provide sound spatial bounds. Preserve the
    // old distance/min behavior for those triangles by always evaluating them.
    unbounded: Vec<Triangle>,
    nodes: Vec<Node>,
}

impl TriangleQuery {
    pub(super) fn new(vertices: &[Vertex], indices: &[u32], triangle_cap: usize) -> Self {
        let mut query = Self {
            triangles: Vec::with_capacity((indices.len() / 3).min(triangle_cap)),
            unbounded: Vec::new(),
            nodes: Vec::new(),
        };
        // Cap before rejecting bad indices: an invalid triangle still consumed
        // one of the original bake's first 1024 triangle slots.
        for indices in indices.chunks_exact(3).take(triangle_cap) {
            let (Some(a), Some(b), Some(c)) = (
                vertices.get(indices[0] as usize),
                vertices.get(indices[1] as usize),
                vertices.get(indices[2] as usize),
            ) else {
                continue;
            };
            let triangle = Triangle {
                a: Vec3::from_array(a.position),
                b: Vec3::from_array(b.position),
                c: Vec3::from_array(c.position),
            };
            if triangle.a.is_finite() && triangle.b.is_finite() && triangle.c.is_finite() {
                query.triangles.push(triangle);
            } else {
                query.unbounded.push(triangle);
            }
        }
        if !query.triangles.is_empty() {
            query.build(0..query.triangles.len());
        }
        query
    }

    fn build(&mut self, range: std::ops::Range<usize>) -> usize {
        let mut min = Vec3::splat(f32::INFINITY);
        let mut max = Vec3::splat(f32::NEG_INFINITY);
        for triangle in &self.triangles[range.clone()] {
            min = min.min(triangle.min());
            max = max.max(triangle.max());
        }
        let index = self.nodes.len();
        self.nodes.push(Node {
            min,
            max,
            triangles: range.clone(),
            children: None,
        });
        if range.len() > LEAF_TRIANGLES {
            let extent = max - min;
            let axis = if extent.x >= extent.y && extent.x >= extent.z {
                0
            } else if extent.y >= extent.z {
                1
            } else {
                2
            };
            let half = range.len() / 2;
            self.triangles[range.clone()].select_nth_unstable_by(half, |a, b| {
                // Halve before adding so a large finite coordinate cannot
                // overflow just from computing the sort key.
                let center = |t: &Triangle| t.min()[axis] * 0.5 + t.max()[axis] * 0.5;
                center(a).total_cmp(&center(b))
            });
            let middle = range.start + half;
            let left = self.build(range.start..middle);
            let right = self.build(middle..range.end);
            self.nodes[index].children = Some((left, right));
        }
        index
    }

    pub(super) fn distance(&self, p: Vec3) -> f32 {
        let mut best = f32::MAX;
        for triangle in &self.unbounded {
            best = best.min(triangle.distance(p));
        }
        if !p.is_finite() {
            for triangle in &self.triangles {
                best = best.min(triangle.distance(p));
            }
        } else if !self.nodes.is_empty() {
            self.visit(0, p, &mut best);
        }
        best
    }

    fn visit(&self, index: usize, p: Vec3, best: &mut f32) {
        let node = &self.nodes[index];
        let best_squared = (*best as f64) * (*best as f64);
        // Strict, conservative rejection retains ties and near-boundary points.
        if node.lower_bound_squared(p) > best_squared * (1.0 + 32.0 * f32::EPSILON as f64) {
            return;
        }
        if let Some((left, right)) = node.children {
            let (near, far) = if self.nodes[left].lower_bound_squared(p)
                <= self.nodes[right].lower_bound_squared(p)
            {
                (left, right)
            } else {
                (right, left)
            };
            self.visit(near, p, best);
            self.visit(far, p, best);
        } else {
            for triangle in &self.triangles[node.triangles.clone()] {
                *best = best.min(triangle.distance(p));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn vertex(position: [f32; 3]) -> Vertex {
        Vertex {
            position,
            normal: [0.0, 1.0, 0.0],
            uv: [0.0; 2],
        }
    }

    // Independent reference keeps the old capped index walk, so the test detects
    // changed cap/invalid-index semantics as well as incorrect spatial pruning.
    fn brute(vertices: &[Vertex], indices: &[u32], cap: usize, p: Vec3) -> f32 {
        let mut best = f32::MAX;
        for i in 0..(indices.len() / 3).min(cap) {
            let ia = indices[i * 3] as usize;
            let ib = indices[i * 3 + 1] as usize;
            let ic = indices[i * 3 + 2] as usize;
            if ia < vertices.len() && ib < vertices.len() && ic < vertices.len() {
                best = best.min(point_triangle_distance(
                    p,
                    Vec3::from_array(vertices[ia].position),
                    Vec3::from_array(vertices[ib].position),
                    Vec3::from_array(vertices[ic].position),
                ));
            }
        }
        best
    }

    #[test]
    fn accelerated_sample_matches_brute_force_including_bake_and_edge_cases() {
        let mut vertices = Vec::new();
        let mut indices = Vec::new();
        // Many separated, sloped and differently sized triangles force interior
        // BVH nodes and test points on faces, edges, vertices and outside bounds.
        for i in 0..96 {
            let x = (i % 12) as f32 * 2.3 - 14.0;
            let z = (i / 12) as f32 * 1.7 - 6.0;
            let y = (i % 5) as f32 * 0.21;
            let base = vertices.len() as u32;
            vertices.extend([
                vertex([x, y, z]),
                vertex([x + 0.9, y + 0.2, z]),
                vertex([x, y, z + 0.6]),
            ]);
            indices.extend([base, base + 1, base + 2]);
        }
        indices.extend([0, 0, 0, 0, 1, 1, u32::MAX, 0, 1]);
        let nan = vertices.len() as u32;
        vertices.push(vertex([f32::NAN, 0.0, 0.0]));
        indices.extend([nan, 0, 1, 0, 1]); // Non-finite triangle and incomplete tail.
        let points = [
            Vec3::ZERO,
            Vec3::new(-14.0, 0.0, -6.0),
            Vec3::new(-13.55, 0.1, -6.0),
            Vec3::new(-13.8, 0.04, -5.8),
            Vec3::new(8.3, 0.000001, 3.2),
            Vec3::new(1000.0, -200.0, 900.0),
            Vec3::splat(f32::NAN),
            Vec3::splat(f32::INFINITY),
        ];
        for cap in [0, 1, 17, super::super::MESH_SDF_TRI_CAP] {
            let query = TriangleQuery::new(&vertices, &indices, cap);
            for p in points {
                assert_eq!(
                    query.distance(p).to_bits(),
                    brute(&vertices, &indices, cap, p).to_bits(),
                    "cap={cap}, p={p:?}"
                );
            }
        }
        assert_eq!(
            TriangleQuery::new(&[], &[], 1024).distance(Vec3::ZERO),
            f32::MAX
        );
        assert_eq!(
            TriangleQuery::new(&vertices, &[u32::MAX, 0, 1, 0, 1, 2], 1).distance(Vec3::ZERO),
            f32::MAX
        );

        // Compare all 4096 samples of an actual brick, including unchanged brick
        // bounds and the existing unsigned distance primitive.
        vertices.pop();
        indices.truncate(96 * 3);
        let brick = super::super::bake_mesh_sdf(&vertices, &indices).unwrap();
        let min = Vec3::from_array(brick.min);
        let extent = Vec3::from_array(brick.max) - min;
        let n = super::super::MESH_SDF_BRICK;
        for z in 0..n {
            for y in 0..n {
                for x in 0..n {
                    let p = min
                        + ((Vec3::new(x as f32, y as f32, z as f32) + Vec3::splat(0.5)) / n as f32)
                            * extent;
                    assert_eq!(
                        brick.dist[(z * n * n + y * n + x) as usize].to_bits(),
                        brute(&vertices, &indices, super::super::MESH_SDF_TRI_CAP, p).to_bits(),
                        "brick cell {x},{y},{z}"
                    );
                }
            }
        }
    }
}
