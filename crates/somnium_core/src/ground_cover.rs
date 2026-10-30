//! Procedural near-camera ground cover (Rimeborn's WS1-13): grass, flowers and the like generated per 16 m cell
//! around where the view looks (WS1-19: an isometric camera stands 60 m behind its focus) from a project foliage kind's `ground_cover` rules and the terrain's splat layer, never stored.
//!
//! A painted sidecar holds at most a million instances per terrain, which over a 16 km² tile is one per 16 m². Ground
//! cover only matters near the camera, so it is made there: each cell's instances come from a hash of the terrain,
//! the kind and the cell, so they are the same every time the cell comes back. A kind gets a candidate per
//! `1 / density` m², kept with the probability of its layer's weight (none below `min_layer_weight`) and only on ground
//! no steeper than `max_slope_deg` (the kind's brush rules). Cells beyond the radius are dropped; a terrain edit
//! regenerates. A project without `ground_cover` kinds generates nothing. With `align_to_ground` each instance is
//! tilted to the slope under it, so a patch neither floats nor sinks on a hillside.

use std::collections::HashMap;

use glam::{Quat, Vec3};
use somnium_renderer::terrain::TerrainData;
use somnium_renderer::terrain::foliage_paint::PaintedFoliage;

use crate::foliage_palette::ProjectEntry;

/// Cell edge, metres.
pub const CELL: f32 = 16.0;
/// Most cells generated in one frame (the rest come next frame).
const CELLS_PER_FRAME: usize = 24;

#[derive(Default)]
struct TerrainCover {
    revision: u64,
    /// The area cover grows in (`TerrainStreaming::own_area`), if not the whole terrain.
    area: Option<glam::Vec4>,
    cells: HashMap<(u8, i32, i32), Vec<Cover>>,
    /// Every live cell's instances, rebuilt when the set of cells changes.
    flat: Vec<Cover>,
}

/// One generated instance: where it stands (its yaw and tilt as a painted one's) and its whole rotation.
#[derive(Clone, Copy)]
pub struct Cover {
    /// The instance, terrain-local.
    pub instance: PaintedFoliage,
    /// `Ry(yaw) * Rx(tilt)`, turned onto the ground's normal for `align_to_ground` kinds.
    pub rotation: Quat,
}

/// Generated ground cover, per terrain.
#[derive(Default)]
pub struct GroundCover {
    terrains: HashMap<u32, TerrainCover>,
}

impl GroundCover {
    /// Bring `terrain`'s cover up to date round `camera` (terrain-local: where the view looks, on the ground).
    pub(crate) fn update(
        &mut self,
        terrain_id: u32,
        terrain: &TerrainData,
        palette: &std::collections::BTreeMap<u8, ProjectEntry>,
        camera: Vec3,
        area: Option<glam::Vec4>,
    ) {
        let kinds: Vec<(u8, &ProjectEntry, f32, f32)> = palette
            .iter()
            .filter_map(|(kind, entry)| entry.ground_cover.as_ref().map(|g| (*kind, entry, g.density, g.radius)))
            .collect();
        if kinds.is_empty() {
            self.terrains.remove(&terrain_id);
            return;
        }
        let cover = self.terrains.entry(terrain_id).or_default();
        if cover.revision != terrain.edit_revision || cover.area != area {
            *cover = TerrainCover { revision: terrain.edit_revision, area, ..TerrainCover::default() };
        }
        let mut changed = false;
        let mut made = 0;
        for &(kind, entry, density, radius) in &kinds {
            let before = cover.cells.len();
            cover.cells.retain(|&(k, cx, cz), _| k != kind || cell_distance(cx, cz, camera) <= radius + CELL);
            changed |= cover.cells.len() != before;
            let reach = (radius / CELL).ceil() as i32 + 1;
            let (ox, oz) = ((camera.x / CELL).floor() as i32, (camera.z / CELL).floor() as i32);
            let mut wanted: Vec<(f32, i32, i32)> = (-reach..=reach)
                .flat_map(|dz| (-reach..=reach).map(move |dx| (ox + dx, oz + dz)))
                .map(|(cx, cz)| (cell_distance(cx, cz, camera), cx, cz))
                .filter(|&(d, cx, cz)| d <= radius && !cover.cells.contains_key(&(kind, cx, cz)))
                .collect();
            wanted.sort_by(|a, b| a.0.total_cmp(&b.0));
            for (_, cx, cz) in wanted {
                if made >= CELLS_PER_FRAME {
                    break;
                }
                cover.cells.insert((kind, cx, cz), generate(terrain_id, terrain, kind, entry, density, cx, cz, area));
                made += 1;
                changed = true;
            }
        }
        if changed {
            cover.flat.clear();
            for cell in cover.cells.values() {
                cover.flat.extend_from_slice(cell);
            }
        }
    }

    /// `terrain`'s generated instances (terrain-local), as painted ones are.
    #[must_use]
    pub fn instances(&self, terrain_id: u32) -> &[Cover] {
        self.terrains.get(&terrain_id).map_or(&[], |c| c.flat.as_slice())
    }

    /// Forget a terrain (or every terrain, on a scene change).
    pub fn clear(&mut self) {
        self.terrains.clear();
    }
}

/// Horizontal distance from `camera` to the nearest point of a cell.
fn cell_distance(cx: i32, cz: i32, camera: Vec3) -> f32 {
    let (x0, z0) = (cx as f32 * CELL, cz as f32 * CELL);
    let dx = (x0 - camera.x).max(camera.x - (x0 + CELL)).max(0.0);
    let dz = (z0 - camera.z).max(camera.z - (z0 + CELL)).max(0.0);
    dx.hypot(dz)
}

/// A small counter hash (splitmix64), so a cell's instances never depend on what was generated before it.
fn mix(mut x: u64) -> u64 {
    x = x.wrapping_add(0x9E37_79B9_7F4A_7C15);
    x = (x ^ (x >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    x = (x ^ (x >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    x ^ (x >> 31)
}

fn unit(h: u64) -> f32 {
    (h >> 40) as f32 / (1u64 << 24) as f32
}

#[allow(clippy::too_many_arguments)]
fn generate(
    terrain_id: u32,
    terrain: &TerrainData,
    kind: u8,
    entry: &ProjectEntry,
    density: f32,
    cx: i32,
    cz: i32,
    area: Option<glam::Vec4>,
) -> Vec<Cover> {
    let b = &entry.brush;
    let [wx, wz] = terrain.desc.world_size();
    let seed = mix(u64::from(terrain_id) << 40 ^ u64::from(kind) << 32 ^ (cx as u32 as u64) << 16 ^ (cz as u32 as u64));
    let expected = density * CELL * CELL;
    let count = expected.floor() as u64 + u64::from(unit(mix(seed)) < expected.fract());
    let min_slope_cos = b.max_slope_deg.to_radians().cos();
    let mut out = Vec::new();
    for i in 0..count {
        let h = mix(seed ^ (i + 1).wrapping_mul(0xA24B_AED4_963E_E407));
        let (x, z) = ((cx as f32 + unit(h)) * CELL, (cz as f32 + unit(mix(h))) * CELL);
        if !(0.0..wx).contains(&x) || !(0.0..wz).contains(&z) || !area.is_none_or(|a| inside(a, x, z)) {
            continue;
        }
        let s = terrain.surface_sample(x, z, b.layer);
        let keep = unit(mix(h ^ 1));
        if s.layer_weight < b.min_layer_weight.max(1e-3) || keep > s.layer_weight || s.slope_cos < min_slope_cos {
            continue;
        }
        let t = unit(mix(h ^ 2));
        let instance = PaintedFoliage {
            kind,
            position: Vec3::new(x, s.height, z),
            yaw: unit(mix(h ^ 3)) * std::f32::consts::TAU,
            tilt: b.max_tilt_deg.to_radians() * unit(mix(h ^ 4)),
            scale: b.scale_min + (b.scale_max - b.scale_min) * t,
        };
        let mut rotation = Quat::from_rotation_y(instance.yaw) * Quat::from_rotation_x(instance.tilt);
        if entry.ground_cover.as_ref().is_some_and(|g| g.align_to_ground) {
            rotation = Quat::from_rotation_arc(Vec3::Y, ground_normal(terrain, x, z)) * rotation;
        }
        out.push(Cover { instance, rotation });
    }
    out
}

/// Whether a terrain-local point is in an own area (x0, z0, x1, z1): a window owns its lower edges, not its upper.
fn inside(area: glam::Vec4, x: f32, z: f32) -> bool {
    (area.x..area.z).contains(&x) && (area.y..area.w).contains(&z)
}

/// The ground's normal at a terrain-local point, from the heights a cell either side.
fn ground_normal(terrain: &TerrainData, x: f32, z: f32) -> Vec3 {
    let d = terrain.desc.cell_size;
    let hx = terrain.world_height_at(x + d, z) - terrain.world_height_at(x - d, z);
    let hz = terrain.world_height_at(x, z + d) - terrain.world_height_at(x, z - d);
    Vec3::new(-hx, 2.0 * d, -hz).try_normalize().unwrap_or(Vec3::Y)
}

/// Where the view looks, on the ground: the view's forward ray (from `camera`, terrain-local) met with the ground
/// height under the camera, at most `reach` metres out. A view looking level or up keeps the camera's own place.
#[must_use]
pub fn view_focus(camera: Vec3, forward: Vec3, ground: f32, reach: f32) -> Vec3 {
    if forward.y > -0.05 || camera.y <= ground {
        return camera;
    }
    let t = ((camera.y - ground) / -forward.y).min(reach);
    camera + forward * t
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cells_are_measured_to_their_nearest_edge() {
        let camera = Vec3::new(8.0, 0.0, 8.0);
        assert_eq!(cell_distance(0, 0, camera), 0.0);
        assert!((cell_distance(2, 0, camera) - 24.0).abs() < 1e-4);
        assert!((cell_distance(-1, -1, camera) - 8.0f32.hypot(8.0)).abs() < 1e-4);
    }

    #[test]
    fn an_isometric_view_grows_cover_where_it_looks() {
        // 90 m from the focus, 45 degrees down: the focus is ~64 m ahead of the camera along the ground.
        let forward = Vec3::new(0.0, -1.0, -1.0).normalize();
        let camera = Vec3::new(0.0, 10.0 + 63.64, 0.0);
        let focus = view_focus(camera, forward, 10.0, 400.0);
        assert!((focus - Vec3::new(0.0, 10.0, -63.64)).length() < 0.05, "{focus}");
        assert_eq!(view_focus(camera, Vec3::NEG_Z, 10.0, 400.0), camera, "a level view keeps the camera's place");
        assert!((view_focus(camera, forward, 10.0, 20.0) - camera).length() <= 20.0 + 1e-3, "never past reach");
    }

    #[test]
    fn neighbouring_windows_never_both_own_a_point() {
        // Two 4,096 m windows on a 4,000 m grid, each owning up to the middle of the overlap (WS1-20).
        let west = glam::Vec4::new(0.0, 0.0, 4048.0, 4096.0);
        let east = glam::Vec4::new(48.0, 0.0, 4096.0, 4096.0);
        for x in [3990.0, 4047.9, 4048.0, 4050.0] {
            assert!(inside(west, x + 0.0, 10.0) != inside(east, x - 4000.0, 10.0), "{x}");
        }
    }

    #[test]
    fn the_hash_spreads_and_repeats() {
        let a: Vec<f32> = (0..1000).map(|i| unit(mix(i))).collect();
        let mean = a.iter().sum::<f32>() / a.len() as f32;
        assert!((mean - 0.5).abs() < 0.05, "{mean}");
        assert!(a.iter().all(|v| (0.0..1.0).contains(v)));
        assert_eq!(unit(mix(42)), unit(mix(42)));
    }
}
