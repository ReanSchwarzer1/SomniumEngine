//! Conservative layered heightfield bake and tiled navigation queries.
//!
//! Resolution is an explicit quality/cost choice. Geometry is sampled at the
//! center and inset corners of each voxel column; all samples must support a
//! walkable surface. The resulting cells are convex quads with two triangles,
//! preserving holes and stacked floors without a physics-engine dependency.

use glam::{Vec2, Vec3};
use serde::{Deserialize, Serialize};
use somnium_jobs::{JobDesc, JobError, JobHandle, JobSystem};
use std::cmp::Ordering;
use std::collections::{BTreeMap, BTreeSet, BinaryHeap, VecDeque};

pub type CellId = [i64; 3];
pub type Triangle = [[f32; 3]; 3];
const EPS: f32 = 0.0001;
const MAX_COLUMNS: usize = 262_144;

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Bounds {
    pub min: [f32; 3],
    pub max: [f32; 3],
}
impl Bounds {
    pub fn valid(self) -> bool {
        let (a, b) = (Vec3::from(self.min), Vec3::from(self.max));
        a.is_finite() && b.is_finite() && a.cmplt(b).all()
    }
    pub fn intersects(self, other: Self) -> bool {
        (0..3).all(|i| self.min[i] <= other.max[i] && self.max[i] >= other.min[i])
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct BakeSettings {
    pub voxel_size: f32,
    pub agent_height: f32,
    pub agent_radius: f32,
    pub max_step: f32,
    pub max_slope_degrees: f32,
}
impl Default for BakeSettings {
    fn default() -> Self {
        Self {
            voxel_size: 0.5,
            agent_height: 1.8,
            agent_radius: 0.3,
            max_step: 0.4,
            max_slope_degrees: 45.0,
        }
    }
}
impl BakeSettings {
    fn valid(self) -> bool {
        [
            self.voxel_size,
            self.agent_height,
            self.agent_radius,
            self.max_step,
            self.max_slope_degrees,
        ]
        .into_iter()
        .all(f32::is_finite)
            && self.voxel_size > 0.0
            && self.agent_height > 0.0
            && self.agent_radius >= 0.0
            && self.max_step >= 0.0
            && (0.0..90.0).contains(&self.max_slope_degrees)
    }
}

#[derive(Clone, Debug)]
pub struct BakeInput {
    pub cell: CellId,
    /// Interior ownership volume. Supply geometry outside it by at least the
    /// agent radius so neighboring tile borders are not mistaken for walls.
    pub bounds: Bounds,
    pub triangles: Vec<Triangle>,
    pub obstacles: Vec<Bounds>,
    pub settings: BakeSettings,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct NavPolygon {
    pub vertices: [[f32; 3]; 4],
    pub region: u32,
}
impl NavPolygon {
    fn center(&self) -> Vec3 {
        self.vertices.iter().map(|v| Vec3::from(*v)).sum::<Vec3>() * 0.25
    }
    fn closest(&self, point: Vec3) -> Vec3 {
        let a = Vec3::from(self.vertices[0]);
        let c = Vec3::from(self.vertices[2]);
        Vec3::new(point.x.clamp(a.x, c.x), a.y, point.z.clamp(a.z, c.z))
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct NavTile {
    version: u32,
    cell: CellId,
    bounds: Bounds,
    settings: BakeSettings,
    polygons: Vec<NavPolygon>,
    /// Region contour boundary segments, including hole boundaries.
    contours: Vec<[[f32; 3]; 2]>,
}
impl NavTile {
    pub fn cell(&self) -> CellId {
        self.cell
    }
    pub fn polygons(&self) -> &[NavPolygon] {
        &self.polygons
    }
    pub fn contours(&self) -> &[[[f32; 3]; 2]] {
        &self.contours
    }
    pub fn bounds(&self) -> Bounds {
        self.bounds
    }
    pub fn triangles(&self) -> impl Iterator<Item = Triangle> + '_ {
        self.polygons.iter().flat_map(|p| {
            [
                [p.vertices[0], p.vertices[1], p.vertices[2]],
                [p.vertices[0], p.vertices[2], p.vertices[3]],
            ]
        })
    }
    pub fn encode(&self) -> Result<Vec<u8>, String> {
        serde_json::to_vec(self).map_err(|e| e.to_string())
    }
    pub fn decode(bytes: &[u8]) -> Result<Self, String> {
        if bytes.len() > 128 * 1024 * 1024 {
            return Err("navigation artifact exceeds limit".into());
        }
        let tile: Self = serde_json::from_slice(bytes).map_err(|e| e.to_string())?;
        tile.validate()?;
        Ok(tile)
    }
    fn validate(&self) -> Result<(), String> {
        if self.version != 1
            || !self.bounds.valid()
            || !self.settings.valid()
            || self.polygons.len() > MAX_COLUMNS * 16
        {
            return Err("invalid navigation tile header".into());
        }
        for p in &self.polygons {
            let [a, b, c, d] = p.vertices.map(Vec3::from);
            if !p.vertices.iter().flatten().all(|v| v.is_finite())
                || p.region == 0
                || a.x >= c.x
                || a.z >= c.z
                || b != Vec3::new(c.x, a.y, a.z)
                || d != Vec3::new(a.x, a.y, c.z)
                || c.y != a.y
                || a.x < self.bounds.min[0] - EPS
                || c.x > self.bounds.max[0] + EPS
                || a.z < self.bounds.min[2] - EPS
                || c.z > self.bounds.max[2] + EPS
                || a.y < self.bounds.min[1]
                || a.y >= self.bounds.max[1]
            {
                return Err("invalid navigation polygon".into());
            }
        }
        if !self
            .contours
            .iter()
            .flatten()
            .flatten()
            .all(|v| v.is_finite())
        {
            return Err("invalid contour".into());
        }
        Ok(())
    }
}

fn sample_height(tri: Triangle, p: Vec2) -> Option<f32> {
    let [a, b, c] = tri.map(Vec3::from);
    let v0 = Vec2::new(b.x - a.x, b.z - a.z);
    let v1 = Vec2::new(c.x - a.x, c.z - a.z);
    let v2 = p - Vec2::new(a.x, a.z);
    let den = v0.perp_dot(v1);
    if den.abs() < EPS {
        return None;
    }
    let u = v2.perp_dot(v1) / den;
    let v = v0.perp_dot(v2) / den;
    (u >= -EPS && v >= -EPS && u + v <= 1.0 + EPS)
        .then_some(a.y + u * (b.y - a.y) + v * (c.y - a.y))
}

/// Bake synchronously for command-line cook tools and deterministic tests.
pub fn bake(input: &BakeInput) -> Result<NavTile, String> {
    bake_checked(input, || Ok(()))
}

fn bake_checked(
    input: &BakeInput,
    check: impl Fn() -> Result<(), String>,
) -> Result<NavTile, String> {
    let s = input.settings;
    if !input.bounds.valid()
        || !s.valid()
        || input.obstacles.iter().any(|b| !b.valid())
        || input
            .triangles
            .iter()
            .flatten()
            .flatten()
            .any(|v| !v.is_finite())
    {
        return Err("invalid navigation bake input".into());
    }
    let size = s.voxel_size;
    let nx = ((input.bounds.max[0] - input.bounds.min[0]) / size).ceil() as usize;
    let nz = ((input.bounds.max[2] - input.bounds.min[2]) / size).ceil() as usize;
    if nx.checked_mul(nz).is_none_or(|n| n > MAX_COLUMNS) {
        return Err("navigation tile exceeds voxel budget".into());
    }
    let padding = (s.agent_radius / size).ceil() as i32;
    if padding > 64 {
        return Err("agent radius exceeds tile padding budget".into());
    }
    let slope = s.max_slope_degrees.to_radians().cos();
    let walkable: Vec<_> = input
        .triangles
        .iter()
        .copied()
        .filter(|t| {
            let [a, b, c] = t.map(Vec3::from);
            (b - a).cross(c - a).normalize_or_zero().y.abs() >= slope
        })
        .collect();
    // Steep geometry, including vertical triangles with zero XZ projected
    // area, occupies body space. Its conservative bounds prevent thin walls
    // from vanishing from the heightfield merely because they miss a sample.
    let walls: Vec<Bounds> = input
        .triangles
        .iter()
        .filter_map(|t| {
            let [a, b, c] = t.map(Vec3::from);
            if (b - a).cross(c - a).normalize_or_zero().y.abs() >= slope {
                return None;
            }
            Some(Bounds {
                min: (a.min(b).min(c) - Vec3::splat(EPS)).to_array(),
                max: (a.max(b).max(c) + Vec3::splat(EPS)).to_array(),
            })
        })
        .collect();
    let mut spans: BTreeMap<(i32, i32), Vec<f32>> = BTreeMap::new();
    for z in -padding..nz as i32 + padding {
        check()?;
        for x in -padding..nx as i32 + padding {
            let px = input.bounds.min[0] + (x as f32 + 0.5) * size;
            let pz = input.bounds.min[2] + (z as f32 + 0.5) * size;
            let mut heights: Vec<f32> = walkable
                .iter()
                .filter_map(|&t| sample_height(t, Vec2::new(px, pz)))
                .collect();
            heights.sort_by(f32::total_cmp);
            heights.dedup_by(|a, b| (*a - *b).abs() < EPS);
            heights.retain(|&y| {
                if !(input.bounds.min[1]..input.bounds.max[1]).contains(&y) {
                    return false;
                }
                // All corners need a continuous supporting surface. Nearby
                // slopes may differ in height but must stay inside slope rise.
                let rise = (s.max_slope_degrees.to_radians().tan() * size).max(EPS);
                for (dx, dz) in [
                    (-0.499, -0.499),
                    (0.499, -0.499),
                    (0.499, 0.499),
                    (-0.499, 0.499),
                    (0.0, 0.0),
                ] {
                    let p = Vec2::new(px + dx * size, pz + dz * size);
                    let Some(floor) = walkable
                        .iter()
                        .filter_map(|&t| sample_height(t, p))
                        .filter(|h| (*h - y).abs() <= rise)
                        .min_by(|a, b| (a - y).abs().total_cmp(&(b - y).abs()))
                    else {
                        return false;
                    };
                    if input
                        .triangles
                        .iter()
                        .filter_map(|&t| sample_height(t, p))
                        .any(|h| h > floor + EPS && h < floor + s.agent_height)
                    {
                        return false;
                    }
                }
                !input.obstacles.iter().chain(walls.iter()).any(|b| {
                    px + size * 0.5 + s.agent_radius > b.min[0]
                        && px - size * 0.5 - s.agent_radius < b.max[0]
                        && pz + size * 0.5 + s.agent_radius > b.min[2]
                        && pz - size * 0.5 - s.agent_radius < b.max[2]
                        && y + s.agent_height > b.min[1]
                        && y < b.max[1]
                })
            });
            spans.insert((x, z), heights);
        }
    }
    let mut cells = Vec::new();
    for z in 0..nz as i32 {
        check()?;
        for x in 0..nx as i32 {
            for &y in &spans[&(x, z)] {
                // Erode cliffs/unsupported ledges. Padding samples cross the
                // tile border, so an arbitrary streaming edge stays connected.
                let mut supported = true;
                for dz in -padding..=padding {
                    for dx in -padding..=padding {
                        if (dx * dx + dz * dz) as f32 * size * size
                            > (s.agent_radius + size * 0.5).powi(2)
                        {
                            continue;
                        }
                        if !spans
                            .get(&(x + dx, z + dz))
                            .is_some_and(|h| h.iter().any(|h| (*h - y).abs() <= s.max_step + EPS))
                        {
                            supported = false;
                        }
                    }
                }
                if supported {
                    cells.push((x, z, y));
                }
            }
        }
    }
    let mut polygons: Vec<_> = cells
        .iter()
        .map(|&(x, z, y)| {
            let a = input.bounds.min[0] + x as f32 * size;
            let b = input.bounds.min[2] + z as f32 * size;
            let c = (a + size).min(input.bounds.max[0]);
            let d = (b + size).min(input.bounds.max[2]);
            NavPolygon {
                vertices: [[a, y, b], [c, y, b], [c, y, d], [a, y, d]],
                region: 0,
            }
        })
        .collect();
    // Spatial buckets make region growth O(number of spans), not O(n²).
    let mut columns: BTreeMap<(i32, i32), Vec<usize>> = BTreeMap::new();
    for (i, &(x, z, _)) in cells.iter().enumerate() {
        columns.entry((x, z)).or_default().push(i);
    }
    let mut region = 0;
    for start in 0..polygons.len() {
        if polygons[start].region != 0 {
            continue;
        }
        region += 1;
        polygons[start].region = region;
        let mut queue = VecDeque::from([start]);
        while let Some(i) = queue.pop_front() {
            let (x, z, y) = cells[i];
            for (dx, dz) in [(1, 0), (-1, 0), (0, 1), (0, -1)] {
                for &j in columns.get(&(x + dx, z + dz)).into_iter().flatten() {
                    if polygons[j].region == 0 && (cells[j].2 - y).abs() <= s.max_step + EPS {
                        polygons[j].region = region;
                        queue.push_back(j);
                    }
                }
            }
        }
    }
    let mut contours = Vec::new();
    for (i, &(x, z, y)) in cells.iter().enumerate() {
        for (edge, (dx, dz)) in [(0, -1), (1, 0), (0, 1), (-1, 0)].into_iter().enumerate() {
            let connected = columns.get(&(x + dx, z + dz)).is_some_and(|v| {
                v.iter()
                    .any(|&j| (cells[j].2 - y).abs() <= s.max_step + EPS)
            });
            if !connected {
                contours.push([
                    polygons[i].vertices[edge],
                    polygons[i].vertices[(edge + 1) % 4],
                ]);
            }
        }
    }
    let tile = NavTile {
        version: 1,
        cell: input.cell,
        bounds: input.bounds,
        settings: s,
        polygons,
        contours,
    };
    tile.validate()?;
    Ok(tile)
}

/// Cancellation is polled once per voxel row and region pass.
pub fn submit_bake(
    jobs: &mut JobSystem,
    input: BakeInput,
    desc: JobDesc,
) -> Result<JobHandle<NavTile>, JobError> {
    jobs.submit_with(desc, move |ctx| {
        bake_checked(&input, || {
            ctx.check_cancelled()
                .map_err(|_| "navigation bake cancelled".into())
        })
    })
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct OffMeshLink {
    pub id: u64,
    pub start: Vec3,
    pub end: Vec3,
    pub bidirectional: bool,
    /// Maximum attachment distance to the nearest walkable polygon.
    pub radius: f32,
    /// Nonnegative additional traversal cost, in metres.
    pub cost: f32,
}
#[derive(Clone, Debug, PartialEq)]
pub struct NavPath {
    pub points: Vec<Vec3>,
    pub links: Vec<u64>,
    /// Changes after install/unload; agents replan if their path is stale.
    pub revision: u64,
}
#[derive(Clone, Copy, Debug)]
struct Edge {
    to: usize,
    portal: [Vec3; 2],
    cost: f32,
    link: Option<(u64, Vec3, Vec3)>,
}
#[derive(Default)]
pub struct NavWorld {
    tiles: BTreeMap<CellId, NavTile>,
    links: BTreeMap<u64, OffMeshLink>,
    polygons: Vec<NavPolygon>,
    adjacency: Vec<Vec<Edge>>,
    revision: u64,
}
impl NavWorld {
    pub fn revision(&self) -> u64 {
        self.revision
    }
    pub fn tile(&self, cell: CellId) -> Option<&NavTile> {
        self.tiles.get(&cell)
    }
    pub fn cells(&self) -> impl Iterator<Item = CellId> + '_ {
        self.tiles.keys().copied()
    }
    pub fn install(&mut self, tile: NavTile) {
        self.tiles.insert(tile.cell, tile);
        self.rebuild();
    }
    pub fn unload(&mut self, cell: CellId) -> bool {
        let removed = self.tiles.remove(&cell).is_some();
        if removed {
            self.rebuild();
        }
        removed
    }
    /// Only intersecting cells need rebaking after an obstacle edit. Caller
    /// supplies the union of the old and new obstacle volumes when moving it.
    pub fn affected_cells(&self, obstacle: Bounds) -> Vec<CellId> {
        self.tiles
            .iter()
            .filter_map(|(id, tile)| {
                let r = tile.settings.agent_radius + tile.settings.voxel_size;
                let mut expanded = obstacle;
                for axis in [0, 2] {
                    expanded.min[axis] -= r;
                    expanded.max[axis] += r;
                }
                tile.bounds.intersects(expanded).then_some(*id)
            })
            .collect()
    }
    pub fn set_link(&mut self, link: OffMeshLink) -> Result<(), String> {
        if !link.start.is_finite()
            || !link.end.is_finite()
            || !link.radius.is_finite()
            || link.radius < 0.0
            || !link.cost.is_finite()
            || link.cost < 0.0
        {
            return Err("invalid off-mesh link".into());
        }
        self.links.insert(link.id, link);
        self.rebuild();
        Ok(())
    }
    pub fn remove_link(&mut self, id: u64) {
        if self.links.remove(&id).is_some() {
            self.rebuild();
        }
    }
    fn nearest_index(&self, point: Vec3, max_distance: f32) -> Option<(usize, Vec3)> {
        if !point.is_finite() || !max_distance.is_finite() || max_distance < 0.0 {
            return None;
        }
        self.polygons
            .iter()
            .enumerate()
            .map(|(i, p)| (i, p.closest(point)))
            .filter(|(_, p)| p.distance(point) <= max_distance)
            .min_by(|a, b| {
                a.1.distance_squared(point)
                    .total_cmp(&b.1.distance_squared(point))
            })
    }
    pub fn nearest(&self, point: Vec3, max_distance: f32) -> Option<Vec3> {
        self.nearest_index(point, max_distance).map(|(_, p)| p)
    }
    fn rebuild(&mut self) {
        self.revision = self.revision.wrapping_add(1);
        self.polygons.clear();
        let mut steps = Vec::new();
        for tile in self.tiles.values() {
            steps.extend(std::iter::repeat_n(
                tile.settings.max_step,
                tile.polygons.len(),
            ));
            self.polygons.extend(tile.polygons.iter().cloned());
        }
        self.adjacency = vec![Vec::new(); self.polygons.len()];
        // Bucket coplanar XZ edges independently of height to stitch stacked
        // floors only when their step-height policy allows it.
        type EdgeKey = (i64, i64, i64, i64);
        type PolygonEdges = Vec<(usize, [Vec3; 2])>;
        let mut edges: BTreeMap<EdgeKey, PolygonEdges> = BTreeMap::new();
        let quant = |v: f32| (f64::from(v) * 10_000.0).round() as i64;
        for (i, p) in self.polygons.iter().enumerate() {
            for e in 0..4 {
                let a = Vec3::from(p.vertices[e]);
                let b = Vec3::from(p.vertices[(e + 1) % 4]);
                let aa = (quant(a.x), quant(a.z));
                let bb = (quant(b.x), quant(b.z));
                let (u, v) = if aa < bb { (aa, bb) } else { (bb, aa) };
                let key = (u.0, u.1, v.0, v.1);
                for &(j, _) in edges.get(&key).into_iter().flatten() {
                    if (p.center().y - self.polygons[j].center().y).abs()
                        > steps[i].min(steps[j]) + EPS
                    {
                        continue;
                    }
                    let cost = p.center().distance(self.polygons[j].center());
                    self.adjacency[i].push(Edge {
                        to: j,
                        portal: [a, b],
                        cost,
                        link: None,
                    });
                    self.adjacency[j].push(Edge {
                        to: i,
                        portal: [b, a],
                        cost,
                        link: None,
                    });
                }
                edges.entry(key).or_default().push((i, [a, b]));
            }
        }
        for l in self.links.values() {
            if let (Some((a, start)), Some((b, end))) = (
                self.nearest_index(l.start, l.radius),
                self.nearest_index(l.end, l.radius),
            ) {
                let cost = self.polygons[a].center().distance(start)
                    + start.distance(end)
                    + end.distance(self.polygons[b].center())
                    + l.cost;
                self.adjacency[a].push(Edge {
                    to: b,
                    portal: [start, start],
                    cost,
                    link: Some((l.id, start, end)),
                });
                if l.bidirectional {
                    self.adjacency[b].push(Edge {
                        to: a,
                        portal: [end, end],
                        cost,
                        link: Some((l.id, end, start)),
                    });
                }
            }
        }
    }
    pub fn path(&self, start: Vec3, goal: Vec3, snap_distance: f32) -> Option<NavPath> {
        let (source, start) = self.nearest_index(start, snap_distance)?;
        let (target, goal) = self.nearest_index(goal, snap_distance)?;
        let mut best = vec![f32::INFINITY; self.polygons.len()];
        let mut previous = vec![None; self.polygons.len()];
        let mut open = BinaryHeap::new();
        best[source] = 0.0;
        open.push(Open {
            node: source,
            score: 0.0,
        });
        let target_center = self.polygons[target].center();
        while let Some(Open { node, .. }) = open.pop() {
            if node == target {
                break;
            }
            for &edge in &self.adjacency[node] {
                let cost = best[node] + edge.cost;
                if cost < best[edge.to] {
                    best[edge.to] = cost;
                    previous[edge.to] = Some((node, edge));
                    open.push(Open {
                        node: edge.to,
                        score: cost + self.polygons[edge.to].center().distance(target_center),
                    });
                }
            }
        }
        if !best[target].is_finite() {
            return None;
        }
        let mut corridor = Vec::new();
        let mut cursor = target;
        while cursor != source {
            let (from, edge) = previous[cursor]?;
            corridor.push((from, edge));
            cursor = from;
        }
        corridor.reverse();
        let mut points = vec![start];
        let mut portals = Vec::new();
        let mut links = Vec::new();
        let mut section = start;
        for (from, edge) in corridor {
            if let Some((id, a, b)) = edge.link {
                points.extend(funnel(section, a, &portals).into_iter().skip(1));
                points.push(b);
                links.push(id);
                portals.clear();
                section = b;
            } else {
                let direction = self.polygons[edge.to].center() - self.polygons[from].center();
                let mid = (edge.portal[0] + edge.portal[1]) * 0.5;
                let a = edge.portal[0] - mid;
                let left = direction.x * a.z - direction.z * a.x > 0.0;
                portals.push(if left {
                    edge.portal
                } else {
                    [edge.portal[1], edge.portal[0]]
                });
            }
        }
        points.extend(funnel(section, goal, &portals).into_iter().skip(1));
        points.dedup_by(|a, b| a.distance_squared(*b) < EPS * EPS);
        Some(NavPath {
            points,
            links,
            revision: self.revision,
        })
    }
    /// Walk a line through linked polygons. Returns the first wall position,
    /// or None when the whole segment remains on the same walkable layer.
    pub fn raycast(&self, start: Vec3, end: Vec3, snap_distance: f32) -> Option<Vec3> {
        if !end.is_finite() {
            return Some(start);
        }
        let Some((source, start)) = self.nearest_index(start, snap_distance) else {
            return Some(start);
        };
        let delta = end - start;
        // Clip the line against convex polygons, then traverse only actual
        // adjacency. Zero-length intervals are retained: crossing a grid
        // corner visits a side polygon at one instant before the diagonal.
        let intervals: Vec<_> = self
            .polygons
            .iter()
            .map(|p| {
                let mut low = 0.0_f32;
                let mut high = 1.0_f32;
                let a = Vec3::from(p.vertices[0]);
                let c = Vec3::from(p.vertices[2]);
                for (axis, min, max) in [
                    (0, a.x, c.x),
                    (2, a.z, c.z),
                    (1, a.y - snap_distance - EPS, a.y + snap_distance + EPS),
                ] {
                    if delta[axis].abs() < EPS {
                        if start[axis] < min - EPS || start[axis] > max + EPS {
                            return None;
                        }
                    } else {
                        let t0 = (min - start[axis]) / delta[axis];
                        let t1 = (max - start[axis]) / delta[axis];
                        low = low.max(t0.min(t1));
                        high = high.min(t0.max(t1));
                        if low > high + EPS {
                            return None;
                        }
                    }
                }
                Some((low, high))
            })
            .collect();
        let mut queue = VecDeque::from([source]);
        let mut visited = BTreeSet::from([source]);
        let mut furthest = 0.0_f32;
        while let Some(i) = queue.pop_front() {
            let Some((low, high)) = intervals[i] else {
                continue;
            };
            furthest = furthest.max(high);
            if high >= 1.0 - EPS {
                return None;
            }
            for edge in &self.adjacency[i] {
                if edge.link.is_some() || visited.contains(&edge.to) {
                    continue;
                }
                if intervals[edge.to].is_some_and(|(a, b)| a <= high + EPS && b >= low - EPS) {
                    visited.insert(edge.to);
                    queue.push_back(edge.to);
                }
            }
        }
        Some(start + delta * furthest.clamp(0.0, 1.0))
    }
}

#[derive(Clone, Copy)]
struct Open {
    node: usize,
    score: f32,
}
impl PartialEq for Open {
    fn eq(&self, o: &Self) -> bool {
        self.node == o.node && self.score == o.score
    }
}
impl Eq for Open {}
impl PartialOrd for Open {
    fn partial_cmp(&self, o: &Self) -> Option<Ordering> {
        Some(self.cmp(o))
    }
}
impl Ord for Open {
    fn cmp(&self, o: &Self) -> Ordering {
        o.score
            .total_cmp(&self.score)
            .then_with(|| o.node.cmp(&self.node))
    }
}

fn area(a: Vec3, b: Vec3, c: Vec3) -> f32 {
    (b.x - a.x) * (c.z - a.z) - (b.z - a.z) * (c.x - a.x)
}
/// String-pull a sequence of portals ordered left/right along the corridor.
pub fn funnel(start: Vec3, end: Vec3, portals: &[[Vec3; 2]]) -> Vec<Vec3> {
    let mut all = portals.to_vec();
    all.push([end, end]);
    let mut out = vec![start];
    let (mut apex, mut left, mut right) = (start, start, start);
    let (mut li, mut ri) = (0, 0);
    let mut i = 0;
    while i < all.len() {
        let [nl, nr] = all[i];
        if area(apex, right, nr) >= 0.0 {
            if apex.distance_squared(right) < EPS * EPS || area(apex, left, nr) < 0.0 {
                right = nr;
                ri = i;
            } else {
                out.push(left);
                apex = left;
                right = apex;
                left = apex;
                i = li + 1;
                continue;
            }
        }
        if area(apex, left, nl) <= 0.0 {
            if apex.distance_squared(left) < EPS * EPS || area(apex, right, nl) > 0.0 {
                left = nl;
                li = i;
            } else {
                out.push(right);
                apex = right;
                left = apex;
                right = apex;
                i = ri + 1;
                continue;
            }
        }
        i += 1;
    }
    if out
        .last()
        .is_none_or(|p| p.distance_squared(end) > EPS * EPS)
    {
        out.push(end);
    }
    out
}

#[derive(Clone, Copy, Debug)]
pub struct Neighbor {
    pub position: Vec3,
    pub velocity: Vec3,
    pub radius: f32,
}
/// Desired velocity with arrival slowdown and reciprocal separation. Movement
/// owners still resolve physical contacts; this is local steering, not a solver.
pub fn steer(
    position: Vec3,
    target: Vec3,
    speed: f32,
    radius: f32,
    neighbors: &[Neighbor],
) -> Vec3 {
    if !position.is_finite()
        || !target.is_finite()
        || !speed.is_finite()
        || speed <= 0.0
        || !radius.is_finite()
        || radius <= 0.0
    {
        return Vec3::ZERO;
    }
    let delta = target - position;
    let mut velocity = delta.normalize_or_zero() * speed.min(delta.length());
    for neighbor in neighbors {
        if !neighbor.position.is_finite()
            || !neighbor.velocity.is_finite()
            || !neighbor.radius.is_finite()
            || neighbor.radius < 0.0
        {
            continue;
        }
        let separation = position - neighbor.position - neighbor.velocity * 0.25;
        let safe = radius + neighbor.radius;
        if separation.length() < safe * 2.0 {
            let away = if separation.length_squared() > EPS * EPS {
                separation.normalize()
            } else {
                Vec3::X
            };
            velocity += away * speed * (1.0 - separation.length() / (safe * 2.0));
        }
    }
    velocity.clamp_length_max(speed)
}

/// Path-following state is per agent; tiles and queries remain shared.
#[derive(Clone, Debug)]
pub struct NavAgent {
    pub speed: f32,
    pub radius: f32,
    destination: Option<Vec3>,
    path: Option<NavPath>,
    waypoint: usize,
    active_link: Option<u64>,
    planned_revision: u64,
}
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AgentMove {
    pub velocity: Vec3,
    /// Gameplay owns the ladder/jump animation and physical relocation. A link
    /// request stays active until `complete_link` acknowledges that traversal.
    pub off_mesh: Option<u64>,
    pub arrived: bool,
}
impl NavAgent {
    pub fn new(speed: f32, radius: f32) -> Result<Self, String> {
        if !speed.is_finite() || speed <= 0.0 || !radius.is_finite() || radius <= 0.0 {
            return Err("invalid navigation agent".into());
        }
        Ok(Self {
            speed,
            radius,
            destination: None,
            path: None,
            waypoint: 0,
            active_link: None,
            planned_revision: 0,
        })
    }
    /// Current corridor points and link identities, for gameplay/debug previews.
    pub fn path(&self) -> Option<&NavPath> {
        self.path.as_ref()
    }
    pub fn destination(&self) -> Option<Vec3> {
        self.destination
    }
    pub fn set_destination(&mut self, nav: &NavWorld, position: Vec3, destination: Vec3) -> bool {
        self.destination = Some(destination);
        self.planned_revision = nav.revision();
        self.active_link = None;
        self.path = nav.path(position, destination, self.radius.max(0.25));
        self.waypoint = 1;
        self.path.is_some()
    }
    pub fn complete_link(&mut self, id: u64) -> bool {
        if self.active_link == Some(id) {
            self.active_link = None;
            self.waypoint += 1;
            true
        } else {
            false
        }
    }
    pub fn update(
        &mut self,
        nav: &NavWorld,
        position: Vec3,
        dt: f32,
        neighbors: &[Neighbor],
    ) -> AgentMove {
        let stopped = AgentMove {
            velocity: Vec3::ZERO,
            off_mesh: None,
            arrived: false,
        };
        if !position.is_finite() || !dt.is_finite() || dt <= 0.0 {
            return stopped;
        }
        if self.planned_revision != nav.revision() {
            if let Some(goal) = self.destination {
                self.set_destination(nav, position, goal);
            }
        }
        if let Some(id) = self.active_link {
            return AgentMove {
                off_mesh: Some(id),
                ..stopped
            };
        }
        let Some(path) = &self.path else {
            return stopped;
        };
        while self.waypoint < path.points.len()
            && position.distance(path.points[self.waypoint]) < 0.0001
        {
            self.waypoint += 1;
        }
        if self.waypoint >= path.points.len() {
            return AgentMove {
                arrived: true,
                ..stopped
            };
        }
        let target = path.points[self.waypoint];
        for id in &path.links {
            if let Some(link) = nav.links.get(id) {
                let forward =
                    position.distance(link.start) < self.radius && target.distance(link.end) < EPS;
                let backward = link.bidirectional
                    && position.distance(link.end) < self.radius
                    && target.distance(link.start) < EPS;
                if forward || backward {
                    self.active_link = Some(*id);
                    return AgentMove {
                        off_mesh: Some(*id),
                        ..stopped
                    };
                }
            }
        }
        let steering_target = if self.waypoint + 1 < path.points.len() {
            position
                + (target - position).normalize_or_zero()
                    * self.speed.max(position.distance(target))
        } else {
            target
        };
        let mut velocity = steer(
            position,
            steering_target,
            self.speed,
            self.radius,
            neighbors,
        );
        velocity = velocity.clamp_length_max(position.distance(target) / dt);
        let next = position + velocity * dt;
        if let Some(hit) = nav.raycast(position, next, self.radius.max(0.25)) {
            velocity = (hit - position) / dt;
            if !velocity.is_finite() {
                velocity = Vec3::ZERO;
            }
        }
        AgentMove {
            velocity,
            ..stopped
        }
    }
}
