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
use std::collections::{BTreeMap, BTreeSet, BinaryHeap, HashMap, VecDeque};

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

/// Whether a triangle's footprint on the ground (its XZ projection) overlaps the square of half side `half` centred on
/// `centre`: separating axes X, Z and the triangle's edge normals. A vertical wall projects to a segment; that works too.
fn overlaps_square(tri: Triangle, centre: Vec2, half: f32) -> bool {
    let p = tri.map(|v| Vec2::new(v[0], v[2]) - centre);
    let mut axes = [Vec2::X, Vec2::Y, Vec2::ZERO, Vec2::ZERO, Vec2::ZERO];
    for i in 0..3 {
        let edge = p[(i + 1) % 3] - p[i];
        if edge.length_squared() > EPS * EPS {
            axes[2 + i] = edge.perp().normalize();
        }
    }
    axes.iter().filter(|axis| **axis != Vec2::ZERO).all(|axis| {
        let (lo, hi) = p.iter().fold((f32::MAX, f32::MIN), |(lo, hi), v| {
            let d = v.dot(*axis);
            (lo.min(d), hi.max(d))
        });
        let reach = half * (axis.x.abs() + axis.y.abs());
        hi >= -reach && lo <= reach
    })
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
    // area, occupies body space. Its bounds are a quick reject; a column is
    // blocked only where the triangle's own footprint reaches it, so a wall
    // standing at an angle does not close the open ground in its bounding box.
    let walls: Vec<(Bounds, Triangle)> = input
        .triangles
        .iter()
        .filter_map(|t| {
            let [a, b, c] = t.map(Vec3::from);
            if (b - a).cross(c - a).normalize_or_zero().y.abs() >= slope {
                return None;
            }
            Some((Bounds {
                min: (a.min(b).min(c) - Vec3::splat(EPS)).to_array(),
                max: (a.max(b).max(c) + Vec3::splat(EPS)).to_array(),
            }, *t))
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
                    // A surface no higher than a step is stepped onto, not walked into.
                    if input
                        .triangles
                        .iter()
                        .filter_map(|&t| sample_height(t, p))
                        .any(|h| h > floor + s.max_step + EPS && h < floor + s.agent_height)
                    {
                        return false;
                    }
                }
                let near = |b: &Bounds| {
                    px + size * 0.5 + s.agent_radius > b.min[0]
                        && px - size * 0.5 - s.agent_radius < b.max[0]
                        && pz + size * 0.5 + s.agent_radius > b.min[2]
                        && pz - size * 0.5 - s.agent_radius < b.max[2]
                        && y + s.agent_height > b.min[1]
                        && y < b.max[1]
                };
                // Steep faces no taller than a step (kerbs, stair risers) are climbed.
                !input.obstacles.iter().any(near)
                    && !walls.iter().any(|(b, t)| {
                        near(b)
                            && b.max[1] > y + s.max_step + EPS
                            && overlaps_square(*t, Vec2::new(px, pz), size * 0.5 + s.agent_radius)
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

/// A tile from ground heights alone, for open land, where a triangle [`bake`]
/// samples every terrain triangle for every column. One flat cell of
/// `settings.voxel_size` per column (the columns that fit inside `bounds`),
/// at the height `height(x, z)` gives at its centre: `None` for no ground (a
/// hole, water, off the map); a height outside `bounds` drops the cell (a
/// roof, when heights come from a ray cast down). Neighbours join where their
/// heights differ by no more than a walkable slope (`max_slope_degrees`)
/// rises across a cell, or `max_step` if that is more: hills are walked,
/// cliffs and walls are not. The tile keeps that rise as its `max_step`, so
/// tiles built alike on one grid join each other once installed. Nothing is
/// eroded by the agent's radius (a cell is coarse already): paths touch the
/// corners of what they go round.
pub fn heightfield(
    cell: CellId,
    bounds: Bounds,
    settings: BakeSettings,
    height: impl Fn(f32, f32) -> Option<f32>,
) -> Result<NavTile, String> {
    if !bounds.valid() || !settings.valid() {
        return Err("invalid heightfield input".into());
    }
    let size = settings.voxel_size;
    let count =
        |axis: usize| ((bounds.max[axis] - bounds.min[axis]) / size + 1e-3).floor() as usize;
    let (nx, nz) = (count(0), count(2));
    if nx == 0 || nz == 0 || nx.checked_mul(nz).is_none_or(|n| n > MAX_COLUMNS) {
        return Err("navigation tile exceeds voxel budget".into());
    }
    let rise = (settings.max_slope_degrees.to_radians().tan() * size).max(settings.max_step);
    let settings = BakeSettings {
        max_step: rise,
        ..settings
    };
    let corner = |axis: usize, i: usize| bounds.min[axis] + i as f32 * size;
    let ys: Vec<Option<f32>> = (0..nx * nz)
        .map(|i| {
            let (x, z) = (i % nx, i / nx);
            let centre = |axis: usize, i: usize| corner(axis, i) + 0.5 * size;
            height(centre(0, x), centre(2, z))
                .filter(|y| y.is_finite() && (bounds.min[1]..bounds.max[1]).contains(y))
        })
        .collect();
    let joined = |a: usize, b: usize| match (ys[a], ys[b]) {
        (Some(p), Some(q)) => (p - q).abs() <= rise + EPS,
        _ => false,
    };
    // Across each edge in the polygon's own order: -z, +x, +z, -x.
    let across = |i: usize| {
        let (x, z) = (i % nx, i / nx);
        [
            (z > 0).then(|| i - nx),
            (x + 1 < nx).then(|| i + 1),
            (z + 1 < nz).then(|| i + nx),
            (x > 0).then(|| i - 1),
        ]
    };
    let mut region = vec![0u32; nx * nz];
    let mut regions = 0;
    for start in 0..nx * nz {
        if ys[start].is_none() || region[start] != 0 {
            continue;
        }
        regions += 1;
        region[start] = regions;
        let mut queue = VecDeque::from([start]);
        while let Some(i) = queue.pop_front() {
            for j in across(i).into_iter().flatten() {
                if region[j] == 0 && joined(i, j) {
                    region[j] = regions;
                    queue.push_back(j);
                }
            }
        }
    }
    let mut polygons = Vec::new();
    let mut contours = Vec::new();
    for i in 0..nx * nz {
        let Some(y) = ys[i] else {
            continue;
        };
        let (x, z) = (i % nx, i / nx);
        let (x0, x1, z0, z1) = (
            corner(0, x),
            corner(0, x + 1),
            corner(2, z),
            corner(2, z + 1),
        );
        let p = NavPolygon {
            vertices: [[x0, y, z0], [x1, y, z0], [x1, y, z1], [x0, y, z1]],
            region: region[i],
        };
        for (edge, j) in across(i).into_iter().enumerate() {
            if !j.is_some_and(|j| joined(i, j)) {
                contours.push([p.vertices[edge], p.vertices[(edge + 1) % 4]]);
            }
        }
        polygons.push(p);
    }
    let tile = NavTile {
        version: 1,
        cell,
        bounds,
        settings,
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
/// How a body crosses an off-mesh link, for the gameplay that moves it. Set
/// per link with [`NavWorld::set_link_kind`]; unset is [`LinkKind::Auto`].
/// Path costs do not depend on it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum LinkKind {
    /// Decided by the rise from where the link is entered to where it is
    /// left ([`LinkKind::resolve`]): up past [`LinkKind::LEVEL`] climbs, down
    /// past it drops, level jumps.
    #[default]
    Auto,
    /// Up a ledge or over a wall: run into it holding climb (a vault or a
    /// climb).
    Climb,
    /// Across a gap (or a low wall), level or rising: leap from its edge.
    Jump,
    /// Down: walk off the edge.
    Drop,
    /// Into or out of water: in by dropping or wading in, out by climbing
    /// the bank.
    Swim,
}

impl LinkKind {
    /// Rise or fall within which an [`LinkKind::Auto`] link counts as level.
    pub const LEVEL: f32 = 0.5;

    /// This kind for a link crossed from `from` to `to`, with
    /// [`LinkKind::Auto`] decided by the rise; never `Auto`.
    pub fn resolve(self, from: Vec3, to: Vec3) -> Self {
        match self {
            Self::Auto => {
                let rise = to.y - from.y;
                if rise > Self::LEVEL {
                    Self::Climb
                } else if rise < -Self::LEVEL {
                    Self::Drop
                } else {
                    Self::Jump
                }
            }
            kind => kind,
        }
    }
}

/// One stretch of a [`NavPath`], from a corner to the next.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PathLeg {
    /// Where it starts.
    pub from: Vec3,
    /// Where it ends.
    pub to: Vec3,
    /// The off-mesh link it crosses and how (never [`LinkKind::Auto`]), or
    /// `None` on the mesh.
    pub link: Option<(u64, LinkKind)>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct NavPath {
    pub points: Vec<Vec3>,
    pub links: Vec<u64>,
    /// Changes after install/unload; agents replan if their path is stale.
    pub revision: u64,
}

impl NavPath {
    /// The path as legs, each marked when it crosses one of the path's
    /// links: a leg whose ends sit within the link's radius of the link's
    /// ends (either way for a bidirectional link). Gameplay walking the
    /// corners reads how to cross each link from here.
    pub fn legs(&self, nav: &NavWorld) -> Vec<PathLeg> {
        let links: Vec<&OffMeshLink> = self.links.iter().filter_map(|id| nav.link(*id)).collect();
        let near = |a: Vec3, b: Vec3, r: f32| a.distance(b) <= r + 1e-3;
        self.points
            .windows(2)
            .map(|w| {
                let (from, to) = (w[0], w[1]);
                let link = links
                    .iter()
                    .find(|l| {
                        (near(from, l.start, l.radius) && near(to, l.end, l.radius))
                            || (l.bidirectional
                                && near(from, l.end, l.radius)
                                && near(to, l.start, l.radius))
                    })
                    .map(|l| (l.id, nav.link_kind(l.id).resolve(from, to)));
                PathLeg { from, to, link }
            })
            .collect()
    }
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
    /// By link id; kept when a link is removed and set again (an editor
    /// re-syncing a moved link keeps its kind).
    kinds: BTreeMap<u64, LinkKind>,
    /// Polygons by the ground cells ([`INDEX_CELL`] metres) their extent
    /// touches, so `nearest` looks only near the point.
    index: HashMap<(i32, i32), Vec<u32>>,
    /// Kept between path searches (see [`Search`]).
    search: std::sync::Mutex<Search>,
    /// Each polygon's step (its tile's `max_step`), each tile's run of
    /// polygons, and the polygons holding link edges: what streaming a tile
    /// in or out needs to touch only that tile.
    steps: Vec<f32>,
    spans: BTreeMap<CellId, (usize, usize)>,
    linked: Vec<usize>,
}
type EdgeKey = (i64, i64, i64, i64);
/// The ground-plane key of a polygon's edge, the same either way round.
fn edge_key(a: Vec3, b: Vec3) -> EdgeKey {
    let quant = |v: f32| (f64::from(v) * 10_000.0).round() as i64;
    let aa = (quant(a.x), quant(a.z));
    let bb = (quant(b.x), quant(b.z));
    let (u, v) = if aa < bb { (aa, bb) } else { (bb, aa) };
    (u.0, u.1, v.0, v.1)
}
/// A path search's bookkeeping, kept between searches so one costs what it
/// visits rather than the size of the mesh: an entry counts only while it
/// carries this search's generation.
#[derive(Default)]
struct Search {
    generation: u32,
    stamp: Vec<u32>,
    best: Vec<f32>,
    /// Where each polygon was reached from: the polygon, and the edge's
    /// place in its list.
    previous: Vec<Option<(u32, u32)>>,
}
impl Search {
    fn begin(&mut self, polygons: usize) {
        if self.stamp.len() != polygons {
            *self = Self {
                generation: 0,
                stamp: vec![0; polygons],
                best: vec![0.0; polygons],
                previous: vec![None; polygons],
            };
        }
        self.generation = self.generation.wrapping_add(1);
        if self.generation == 0 {
            self.stamp.fill(0);
            self.generation = 1;
        }
    }
    fn best(&self, i: usize) -> f32 {
        if self.stamp[i] == self.generation {
            self.best[i]
        } else {
            f32::INFINITY
        }
    }
    fn previous(&self, i: usize) -> Option<(u32, u32)> {
        if self.stamp[i] == self.generation {
            self.previous[i]
        } else {
            None
        }
    }
    fn set(&mut self, i: usize, best: f32, previous: Option<(u32, u32)>) {
        self.stamp[i] = self.generation;
        self.best[i] = best;
        self.previous[i] = previous;
    }
}
/// The side of a [`NavWorld`] index cell, metres.
const INDEX_CELL: f32 = 2.0;
fn index_cell(x: f32, z: f32) -> (i32, i32) {
    (
        (x / INDEX_CELL).floor() as i32,
        (z / INDEX_CELL).floor() as i32,
    )
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
    /// An installed link, for a caller walking a [`NavPath`] that names it.
    pub fn link(&self, id: u64) -> Option<&OffMeshLink> {
        self.links.get(&id)
    }
    /// How the link `id` is crossed (before or after it is set);
    /// [`LinkKind::Auto`] forgets it. Paths do not change.
    pub fn set_link_kind(&mut self, id: u64, kind: LinkKind) {
        if kind == LinkKind::Auto {
            self.kinds.remove(&id);
        } else {
            self.kinds.insert(id, kind);
        }
    }
    /// How the link `id` is crossed; [`LinkKind::Auto`] unless set.
    pub fn link_kind(&self, id: u64) -> LinkKind {
        self.kinds.get(&id).copied().unwrap_or_default()
    }
    fn nearest_index(&self, point: Vec3, max_distance: f32) -> Option<(usize, Vec3)> {
        if !point.is_finite() || !max_distance.is_finite() || max_distance < 0.0 {
            return None;
        }
        // In index order, as a scan of every polygon would, so ties go the
        // same way; a polygon within reach has its extent within reach on
        // the ground (a millimetre spare for rounding).
        let best = |ids: &mut dyn Iterator<Item = usize>| {
            ids.map(|i| (i, self.polygons[i].closest(point)))
                .filter(|(_, p)| p.distance(point) <= max_distance)
                .min_by(|a, b| {
                    a.1.distance_squared(point)
                        .total_cmp(&b.1.distance_squared(point))
                })
        };
        let reach = max_distance + 1e-3;
        let lo = index_cell(point.x - reach, point.z - reach);
        let hi = index_cell(point.x + reach, point.z + reach);
        let span =
            (i64::from(hi.0) - i64::from(lo.0) + 1) * (i64::from(hi.1) - i64::from(lo.1) + 1);
        if span > 64 {
            return best(&mut (0..self.polygons.len()));
        }
        let mut ids: Vec<usize> = (lo.0..=hi.0)
            .flat_map(|x| (lo.1..=hi.1).map(move |z| (x, z)))
            .filter_map(|cell| self.index.get(&cell))
            .flatten()
            .map(|&i| i as usize)
            .collect();
        ids.sort_unstable();
        ids.dedup();
        best(&mut ids.into_iter())
    }
    pub fn nearest(&self, point: Vec3, max_distance: f32) -> Option<Vec3> {
        self.nearest_index(point, max_distance).map(|(_, p)| p)
    }
    fn rebuild(&mut self) {
        self.revision = self.revision.wrapping_add(1);
        self.polygons.clear();
        self.steps.clear();
        self.spans.clear();
        self.linked.clear();
        for tile in self.tiles.values() {
            let start = self.polygons.len();
            self.steps.extend(std::iter::repeat_n(
                tile.settings.max_step,
                tile.polygons.len(),
            ));
            self.polygons.extend(tile.polygons.iter().cloned());
            self.spans.insert(tile.cell, (start, self.polygons.len()));
        }
        self.adjacency = vec![Vec::new(); self.polygons.len()];
        self.index.clear();
        for i in 0..self.polygons.len() {
            self.index_polygon(i);
        }
        // Bucket coplanar XZ edges independently of height to stitch stacked
        // floors only when their step-height policy allows it. Only ever
        // looked up, never walked: a hash map gives the same adjacency, sooner.
        let mut edges: HashMap<EdgeKey, Vec<usize>> =
            HashMap::with_capacity(self.polygons.len() * 2);
        for i in 0..self.polygons.len() {
            self.stitch(i, &mut edges, &|_| false);
        }
        self.link_edges();
    }
    /// Put polygon `i` in the nearest-point index.
    fn index_polygon(&mut self, i: usize) {
        let p = &self.polygons[i];
        let (a, c) = (p.vertices[0], p.vertices[2]);
        let (lo, hi) = (
            index_cell(a[0] - 1e-3, a[2] - 1e-3),
            index_cell(c[0] + 1e-3, c[2] + 1e-3),
        );
        for x in lo.0..=hi.0 {
            for z in lo.1..=hi.1 {
                self.index.entry((x, z)).or_default().push(i as u32);
            }
        }
    }
    /// Join polygon `i` to every polygon already in `edges` across a shared
    /// edge (both ways), then put its own edges there. The portal is the edge
    /// as the polygon later in install order (cell order) has it, which is
    /// `i` unless `after(j)` says `j` comes after it.
    fn stitch(
        &mut self,
        i: usize,
        edges: &mut HashMap<EdgeKey, Vec<usize>>,
        after: &dyn Fn(usize) -> bool,
    ) {
        let p = self.polygons[i].clone();
        for e in 0..4 {
            let a = Vec3::from(p.vertices[e]);
            let b = Vec3::from(p.vertices[(e + 1) % 4]);
            let key = edge_key(a, b);
            for &j in edges.get(&key).into_iter().flatten() {
                let q = &self.polygons[j];
                if (p.center().y - q.center().y).abs() > self.steps[i].min(self.steps[j]) + EPS {
                    continue;
                }
                let cost = p.center().distance(q.center());
                // Seen from `j`'s side, when `j` is the later one.
                let side = |f: usize| {
                    let (c, d) = (q.vertices[f], q.vertices[(f + 1) % 4]);
                    (Vec3::from(c), Vec3::from(d))
                };
                let theirs = (0..4).find(|&f| after(j) && edge_key(side(f).0, side(f).1) == key);
                let (a, b) = theirs.map_or((a, b), |f| (side(f).1, side(f).0));
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
            edges.entry(key).or_default().push(i);
        }
    }
    /// Each link as an edge from the polygon nearest its start to the one
    /// nearest its end (and back, both ways), after every polygon's own.
    fn link_edges(&mut self) {
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
                self.linked.push(a);
                if l.bidirectional {
                    self.adjacency[b].push(Edge {
                        to: a,
                        portal: [end, end],
                        cost,
                        link: Some((l.id, end, start)),
                    });
                    self.linked.push(b);
                }
            }
        }
    }
    /// Take every link edge away (they go back, last, after a stream).
    fn strip_links(&mut self) {
        for i in std::mem::take(&mut self.linked) {
            if let Some(list) = self.adjacency.get_mut(i) {
                list.retain(|e| e.link.is_none());
            }
        }
    }
    /// Install `tile` for streaming: the graph [`NavWorld::install`] gives,
    /// for work in proportion to the tile and what already lies under it,
    /// not to the whole world. Its polygons go after everyone else's rather
    /// than in cell order, so where two routes cost exactly the same a search
    /// may pick the other one. A tile of the same cell is replaced.
    pub fn stream_in(&mut self, tile: NavTile) {
        self.stream_out(tile.cell);
        self.strip_links();
        // What is already here, where the tile lies, bucketed by edge.
        let b = tile.bounds;
        let (lo, hi) = (
            index_cell(b.min[0] - 1e-3, b.min[2] - 1e-3),
            index_cell(b.max[0] + 1e-3, b.max[2] + 1e-3),
        );
        let mut under: Vec<usize> = (lo.0..=hi.0)
            .flat_map(|x| (lo.1..=hi.1).map(move |z| (x, z)))
            .filter_map(|cell| self.index.get(&cell))
            .flatten()
            .map(|&i| i as usize)
            .collect();
        under.sort_unstable();
        under.dedup();
        let mut edges: HashMap<EdgeKey, Vec<usize>> = HashMap::new();
        for j in under {
            let p = &self.polygons[j];
            for e in 0..4 {
                let key = edge_key(
                    Vec3::from(p.vertices[e]),
                    Vec3::from(p.vertices[(e + 1) % 4]),
                );
                edges.entry(key).or_default().push(j);
            }
        }
        // Tiles of later cells come after this one when installed.
        let later: Vec<(usize, usize)> = self
            .spans
            .range(tile.cell..)
            .map(|(_, span)| *span)
            .collect();
        let after = |j: usize| later.iter().any(|&(s, e)| (s..e).contains(&j));
        let start = self.polygons.len();
        self.polygons.extend(tile.polygons.iter().cloned());
        self.steps.extend(std::iter::repeat_n(
            tile.settings.max_step,
            tile.polygons.len(),
        ));
        self.adjacency.resize_with(self.polygons.len(), Vec::new);
        for i in start..self.polygons.len() {
            self.index_polygon(i);
            self.stitch(i, &mut edges, &after);
        }
        self.spans.insert(tile.cell, (start, self.polygons.len()));
        self.tiles.insert(tile.cell, tile);
        self.link_edges();
        self.revision = self.revision.wrapping_add(1);
    }
    /// Remove a tile streamed in (or installed): [`NavWorld::unload`]'s
    /// graph, without rebuilding (only renumbering what came after it).
    pub fn stream_out(&mut self, cell: CellId) -> bool {
        let Some((s, e)) = self.spans.remove(&cell) else {
            return false;
        };
        self.tiles.remove(&cell);
        self.strip_links();
        let gone = s..e;
        let shift = |i: usize| if i >= e { i - (e - s) } else { i };
        self.polygons.drain(gone.clone());
        self.steps.drain(gone.clone());
        self.adjacency.drain(gone.clone());
        for list in &mut self.adjacency {
            list.retain(|x| !gone.contains(&x.to));
            for x in list.iter_mut() {
                x.to = shift(x.to);
            }
        }
        for span in self.spans.values_mut() {
            if span.0 >= e {
                *span = (shift(span.0), shift(span.1));
            }
        }
        self.index.retain(|_, list| {
            list.retain(|&i| !gone.contains(&(i as usize)));
            for i in list.iter_mut() {
                *i = shift(*i as usize) as u32;
            }
            !list.is_empty()
        });
        self.link_edges();
        self.revision = self.revision.wrapping_add(1);
        true
    }
    pub fn path(&self, start: Vec3, goal: Vec3, snap_distance: f32) -> Option<NavPath> {
        let (source, start) = self.nearest_index(start, snap_distance)?;
        let (target, goal) = self.nearest_index(goal, snap_distance)?;
        // The kept bookkeeping, or a fresh one when another search holds it.
        let mut kept = self.search.try_lock().ok();
        let mut fresh = Search::default();
        let search = kept.as_deref_mut().unwrap_or(&mut fresh);
        search.begin(self.polygons.len());
        let mut open = BinaryHeap::new();
        search.set(source, 0.0, None);
        open.push(Open {
            node: source,
            score: 0.0,
        });
        let target_center = self.polygons[target].center();
        while let Some(Open { node, .. }) = open.pop() {
            if node == target {
                break;
            }
            for (k, &edge) in self.adjacency[node].iter().enumerate() {
                let cost = search.best(node) + edge.cost;
                if cost < search.best(edge.to) {
                    search.set(edge.to, cost, Some((node as u32, k as u32)));
                    open.push(Open {
                        node: edge.to,
                        score: cost + self.polygons[edge.to].center().distance(target_center),
                    });
                }
            }
        }
        if !search.best(target).is_finite() {
            return None;
        }
        let mut corridor = Vec::new();
        let mut cursor = target;
        while cursor != source {
            let (from, k) = search.previous(cursor)?;
            let from = from as usize;
            corridor.push((from, self.adjacency[from][k as usize]));
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

#[cfg(test)]
mod stream_tests {
    use super::*;

    /// Every edge as what it joins (by the polygons' corners), where and at
    /// what cost: the graph whatever order the polygons are in.
    fn graph(nav: &NavWorld) -> Vec<String> {
        let key = |i: usize| {
            format!(
                "{:?}",
                nav.polygons[i].vertices.map(|v| v.map(f32::to_bits))
            )
        };
        let mut all: Vec<String> = nav
            .adjacency
            .iter()
            .enumerate()
            .flat_map(|(i, list)| {
                list.iter().map(move |e| {
                    format!(
                        "{} {} {:?} {} {:?}",
                        key(i),
                        key(e.to),
                        e.portal.map(|v| v.to_array().map(f32::to_bits)),
                        e.cost.to_bits(),
                        e.link.map(|l| l.0)
                    )
                })
            })
            .collect();
        all.sort();
        all
    }

    fn tile(cell: i64, x: f32, z: f32, floor: f32) -> NavTile {
        heightfield(
            [cell, 0, 0],
            Bounds {
                min: [x, -20.0, z],
                max: [x + 16.0, 20.0, z + 16.0],
            },
            BakeSettings {
                voxel_size: 2.0,
                ..BakeSettings::default()
            },
            |px, pz| Some(floor + 0.15 * px - 0.1 * pz),
        )
        .unwrap()
    }

    /// Streamed in and out in any order, the graph is the one a full install
    /// of the same tiles builds: neighbours, a deck stacked over one, a link,
    /// and a tile replaced in place.
    #[test]
    fn streaming_builds_the_graph_installing_does() {
        let tiles = [
            tile(0, 0.0, 0.0, 0.0),
            tile(1, 16.0, 0.0, 0.0),
            tile(2, 0.0, 16.0, 0.0),
            tile(3, 16.0, 16.0, 0.0),
            tile(4, 0.0, 0.0, 6.0),
        ];
        let link = OffMeshLink {
            id: 9,
            start: Vec3::new(5.0, 0.0, 5.0),
            end: Vec3::new(5.0, 6.0, 5.0),
            bidirectional: true,
            radius: 1.5,
            cost: 2.0,
        };
        let mut streamed = NavWorld::default();
        streamed.set_link(link).unwrap();
        let mut present: Vec<usize> = Vec::new();
        let steps: [(bool, usize); 10] = [
            (true, 3),
            (true, 0),
            (true, 4),
            (true, 1),
            (true, 2),
            (false, 1),
            (true, 0),
            (false, 4),
            (true, 1),
            (false, 3),
        ];
        for (n, (into, which)) in steps.into_iter().enumerate() {
            if into {
                streamed.stream_in(tiles[which].clone());
                if !present.contains(&which) {
                    present.push(which);
                }
            } else {
                assert!(streamed.stream_out(tiles[which].cell));
                present.retain(|&t| t != which);
            }
            let mut installed = NavWorld::default();
            installed.set_link(link).unwrap();
            for &t in &present {
                installed.install(tiles[t].clone());
            }
            assert_eq!(graph(&streamed), graph(&installed), "after step {n}");
            assert_eq!(streamed.polygons.len(), installed.polygons.len());
            for x in 0..40 {
                let p = Vec3::new(x as f32 * 0.83, 1.0, 31.0 - x as f32 * 0.71);
                assert_eq!(
                    streamed.nearest(p, 3.0),
                    installed.nearest(p, 3.0),
                    "nearest {p} after step {n}"
                );
            }
        }
        assert!(!streamed.stream_out([7, 0, 0]), "nothing there");
    }
}
