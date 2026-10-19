//! Painted foliage as instance batches (`somnium_renderer::instance_batch`, open-world render scale, 19 Oct 2026).
//!
//! The per-instance path (`App::submit_foliage`) visits every instance within reach and submits a draw per part per
//! instance per frame: at Rimeborn's budget of a million instances in a 600 m disc that is a hundred thousand draws or
//! more, about 0.25 µs each on the CPU between the visit and the renderer. Here each 32 m ground cell's instances of
//! one kind are one batch, set when the cell comes within the kind's reach and removed when it falls 32 m beyond it.
//! A frame then walks the cells near the camera, and the GPU picks each copy's parts by distance, as the per-instance
//! path would have: the kind's draw distance, a project entry's LOD mesh, and the legacy leaf and heavy-part drops
//! at `lod_distance` and `impostor_distance`.
//!
//! Opt-in with `SOMNIUM_FOLIAGE_BATCHES=1`. A terrain with an LOD falloff curve keeps the per-instance path (the
//! curve scales every copy every frame). What batches do not have is listed in `instance_batch.rs`: no ray-traced
//! foliage, no virtual shadow pages, two-sided raster.

use glam::{Mat4, Vec2};
use somnium_renderer::instance_batch::{BatchPart, InstanceBatch};
use somnium_renderer::terrain::foliage_paint::PaintedFoliage;

/// Ground metres per cell, as `foliage_grid`'s.
const CELL: f32 = 32.0;
/// A cell is kept this far past its kind's reach, so walking along the edge does not set and remove it every frame.
const KEEP: f32 = 32.0;
/// Batch ids in this namespace are foliage's: the top byte.
const ID_SPACE: u64 = 0xF0 << 56;

/// Whether `SOMNIUM_FOLIAGE_BATCHES=1` is set.
pub(crate) fn enabled() -> bool {
    static ON: std::sync::LazyLock<bool> =
        std::sync::LazyLock::new(|| std::env::var("SOMNIUM_FOLIAGE_BATCHES").as_deref() == Ok("1"));
    *ON
}

/// One part of a palette mesh, as `App`'s foliage parts carry it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Piece {
    pub vertex_offset: u32,
    pub index_offset: u32,
    pub index_count: u32,
    pub material_id: u32,
    pub local: Mat4,
    pub is_leaf: bool,
}

/// A kind's distances, as the per-instance path reads them: squared draw and shadow distances (`f32::MAX` for no
/// limit), the squared distance past which a project entry's LOD mesh draws (`f32::INFINITY` for none), and the
/// terrain component's legacy `lod_distance` and `impostor_distance` (0 for none).
#[derive(Clone, Copy, Debug)]
pub(crate) struct KindDistances {
    pub cull_sq: f32,
    pub shadow_sq: f32,
    pub lod_sq: f32,
    pub legacy_lod: f32,
    pub legacy_impostor: f32,
}

/// `d` as a limit: 0 (none) for a squared `f32::MAX` or infinity, else its root.
fn limit(sq: f32) -> f32 {
    if sq >= f32::MAX { 0.0 } else { sq.max(0.0).sqrt() }
}

/// The parts one copy of `kind` draws and between which horizontal distances, the per-instance path's choice
/// written as distance bands: `near` (a LOD mesh from its swap) and `far` (`0` for no limit).
pub(crate) fn kind_parts(kind: u8, near_mesh: &[Piece], lod_mesh: Option<&[Piece]>, d: KindDistances) -> Vec<BatchPart> {
    let reach = limit(d.cull_sq);
    let shadow_far = limit(d.shadow_sq);
    let casts = d.shadow_sq > 0.0;
    // A project entry's LOD mesh past its distance: two bands, the near mesh until it, the LOD mesh from it.
    let swap = match lod_mesh {
        Some(lod) if d.lod_sq.is_finite() => Some((d.lod_sq.max(0.0).sqrt(), lod)),
        _ => None,
    };
    let mut out = Vec::new();
    let mut band = |mesh: &[Piece], near: f32, far: f32| {
        for (piece, legacy_far) in mesh.iter().zip(legacy_limits(kind, mesh, d)) {
            let far = match (far, legacy_far) {
                (0.0, l) => l,
                (f, 0.0) => f,
                (f, l) => f.min(l),
            };
            if far > 0.0 && far <= near {
                continue;
            }
            out.push(BatchPart {
                vertex_offset: piece.vertex_offset,
                index_offset: piece.index_offset,
                index_count: piece.index_count,
                material_id: piece.material_id,
                local: piece.local,
                near,
                far,
                casts_shadow: casts,
                shadow_far,
            });
        }
    };
    match swap {
        Some((at, lod)) => {
            band(near_mesh, 0.0, if reach > 0.0 { at.min(reach) } else { at });
            if reach == 0.0 || at < reach {
                band(lod, at, reach);
            }
        }
        None => band(near_mesh, 0.0, reach),
    }
    out
}

/// Per part of `mesh`, the distance past which the legacy LOD (`kind < 128`) drops it, 0 if never: past
/// `lod_distance` the leaves go (the heaviest part, without leaves), past `impostor_distance` everything but the
/// solid parts (the cheapest, without leaves). Read off `App::submit_foliage`'s `keep_only`/`drop_heavy`.
fn legacy_limits(kind: u8, mesh: &[Piece], d: KindDistances) -> Vec<f32> {
    if kind >= 128 || mesh.len() < 2 {
        return vec![0.0; mesh.len()];
    }
    let has_leaf = mesh.iter().any(|p| p.is_leaf);
    let cheapest = mesh.iter().enumerate().min_by_key(|(_, p)| p.index_count).map(|(i, _)| i);
    let heaviest = mesh.iter().enumerate().max_by_key(|(_, p)| p.index_count).map(|(i, _)| i);
    let first = |a: f32, b: f32| match (a > 0.0, b > 0.0) {
        (true, true) => a.min(b),
        (true, false) => a,
        (false, true) => b,
        (false, false) => 0.0,
    };
    mesh.iter()
        .enumerate()
        .map(|(i, p)| {
            let by_impostor = if has_leaf { p.is_leaf } else { cheapest != Some(i) };
            let by_lod = if has_leaf { p.is_leaf } else { heaviest == Some(i) };
            match (by_lod, by_impostor) {
                (true, true) => first(d.legacy_lod, d.legacy_impostor),
                (true, false) => d.legacy_lod,
                (false, true) => d.legacy_impostor,
                (false, false) => 0.0,
            }
        })
        .collect()
}

/// A terrain's instances by cell and kind.
#[derive(Default)]
struct Cells {
    key: Option<(u64, usize)>,
    min: [f32; 2],
    dims: [i32; 2],
    /// Per cell, `(kind, start, end)` into `order`.
    cells: Vec<Vec<(u8, u32, u32)>>,
    order: Vec<u32>,
}

impl Cells {
    fn refresh(&mut self, revision: u64, instances: &[PaintedFoliage]) {
        let key = (revision, instances.len());
        if self.key == Some(key) {
            return;
        }
        self.key = Some(key);
        let (mut lo, mut hi) = ([f32::MAX; 2], [f32::MIN; 2]);
        for p in instances {
            lo = [lo[0].min(p.position.x), lo[1].min(p.position.z)];
            hi = [hi[0].max(p.position.x), hi[1].max(p.position.z)];
        }
        if instances.is_empty() {
            (lo, hi) = ([0.0; 2], [0.0; 2]);
        }
        self.min = lo;
        self.dims = [((hi[0] - lo[0]) / CELL) as i32 + 1, ((hi[1] - lo[1]) / CELL) as i32 + 1];
        let cell_of = |p: &PaintedFoliage| {
            let cx = ((p.position.x - lo[0]) / CELL) as i32;
            let cz = ((p.position.z - lo[1]) / CELL) as i32;
            (cz * self.dims[0] + cx) as usize
        };
        let mut order: Vec<u32> = (0..instances.len() as u32).collect();
        order.sort_by_key(|&i| {
            let p = &instances[i as usize];
            (cell_of(p), p.kind)
        });
        let mut cells = vec![Vec::new(); (self.dims[0] * self.dims[1]) as usize];
        let mut start = 0;
        while start < order.len() {
            let first = &instances[order[start] as usize];
            let (cell, kind) = (cell_of(first), first.kind);
            let mut end = start + 1;
            while end < order.len() {
                let p = &instances[order[end] as usize];
                if cell_of(p) != cell || p.kind != kind {
                    break;
                }
                end += 1;
            }
            cells[cell].push((kind, start as u32, end as u32));
            start = end;
        }
        (self.cells, self.order) = (cells, order);
    }
}

/// What one frame asks of the renderer.
#[derive(Default)]
pub(crate) struct Changes {
    pub set: Vec<(u64, InstanceBatch)>,
    pub remove: Vec<u64>,
}

/// The foliage batches set on the renderer, by terrain.
#[derive(Default)]
pub(crate) struct FoliageBatches {
    cells: std::collections::HashMap<u32, Cells>,
    /// `(terrain, cell, kind)` -> (signature, frame last needed, copies).
    live: std::collections::HashMap<(u32, u32, u8), (u64, u64, u32)>,
    frame: u64,
    seen: Vec<u32>,
    pub changes: Changes,
}

fn batch_id(terrain: u32, cell: u32, kind: u8) -> u64 {
    ID_SPACE | (u64::from(terrain & 0xFFFF) << 40) | (u64::from(cell) << 8) | u64::from(kind)
}

fn mix(h: &mut u64, word: u32) {
    *h = (*h ^ u64::from(word)).wrapping_mul(0x0100_0000_01b3);
}

impl FoliageBatches {
    /// Start a frame: what the last frame asked is taken already.
    pub(crate) fn begin_frame(&mut self) {
        self.frame += 1;
        self.seen.clear();
        self.changes = Changes::default();
    }

    /// One terrain's frame: set the batches of cells newly within reach (or whose foliage, parts or distances
    /// changed), and keep the rest live. `parts[kind]` is `None` while a kind's mesh is not loaded. Returns the
    /// copies its live batches hold.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn terrain(
        &mut self,
        terrain: u32,
        revision: u64,
        instances: &[PaintedFoliage],
        model: Mat4,
        camera_local: glam::Vec3,
        parts: &[Option<Vec<BatchPart>>; 256],
    ) -> u32 {
        self.seen.push(terrain);
        let cells = self.cells.entry(terrain).or_default();
        cells.refresh(revision, instances);
        // Per kind: its reach and the signature its batches are built to (a new one rebuilds them).
        let mut reach = [f32::NAN; 256];
        let mut signature = [0u64; 256];
        let mut farthest = 0.0f32;
        for (kind, list) in parts.iter().enumerate() {
            let Some(list) = list else { continue };
            let r = if list.iter().any(|p| p.far == 0.0) {
                f32::INFINITY
            } else {
                list.iter().map(|p| p.far).fold(0.0, f32::max)
            };
            reach[kind] = r;
            farthest = farthest.max(r);
            let mut h = 0xcbf2_9ce4_8422_2325u64 ^ revision.rotate_left(13) ^ instances.len() as u64;
            for value in model.to_cols_array() {
                mix(&mut h, value.to_bits());
            }
            for p in list {
                for word in [p.vertex_offset, p.index_offset, p.index_count, p.material_id, u32::from(p.casts_shadow)] {
                    mix(&mut h, word);
                }
                for value in [p.near, p.far, p.shadow_far].into_iter().chain(p.local.to_cols_array()) {
                    mix(&mut h, value.to_bits());
                }
            }
            signature[kind] = h;
        }
        if cells.order.is_empty() {
            return 0;
        }
        let centre = Vec2::new(camera_local.x, camera_local.z);
        let span = farthest + KEEP;
        let cell_range = |v: f32, lo: f32, n: i32| (((v - lo) / CELL).floor() as i32).clamp(0, n - 1);
        let (x0, x1, z0, z1) = if span.is_finite() {
            (
                cell_range(centre.x - span, cells.min[0], cells.dims[0]),
                cell_range(centre.x + span, cells.min[0], cells.dims[0]),
                cell_range(centre.y - span, cells.min[1], cells.dims[1]),
                cell_range(centre.y + span, cells.min[1], cells.dims[1]),
            )
        } else {
            (0, cells.dims[0] - 1, 0, cells.dims[1] - 1)
        };
        for cz in z0..=z1 {
            for cx in x0..=x1 {
                let cell = (cz * cells.dims[0] + cx) as u32;
                let lo = Vec2::new(cells.min[0] + cx as f32 * CELL, cells.min[1] + cz as f32 * CELL);
                let gap = (lo - centre).max(centre - (lo + Vec2::splat(CELL))).max(Vec2::ZERO).length();
                for &(kind, start, end) in &cells.cells[cell as usize] {
                    let r = reach[kind as usize];
                    if r.is_nan() || gap > r + KEEP {
                        continue;
                    }
                    let key = (terrain, cell, kind);
                    let current = self.live.get(&key).map(|&(sig, _, _)| sig);
                    if current.is_none() && gap > r {
                        continue;
                    }
                    let copies;
                    if current != Some(signature[kind as usize]) {
                        let list = parts[kind as usize].as_ref().expect("a kind with a reach has parts");
                        let placements: Vec<Mat4> = cells.order[start as usize..end as usize]
                            .iter()
                            .map(|&i| &instances[i as usize])
                            .filter(|inst| inst.scale > 0.0)
                            .map(|inst| {
                                model
                                    * Mat4::from_scale_rotation_translation(
                                        glam::Vec3::splat(inst.scale),
                                        glam::Quat::from_rotation_y(inst.yaw) * glam::Quat::from_rotation_x(inst.tilt),
                                        inst.position,
                                    )
                            })
                            .collect();
                        copies = placements.len() as u32;
                        self.changes
                            .set
                            .push((batch_id(terrain, cell, kind), InstanceBatch { parts: list.clone(), placements }));
                    } else {
                        copies = self.live[&key].2;
                    }
                    self.live.insert(key, (signature[kind as usize], self.frame, copies));
                }
            }
        }
        self.live
            .iter()
            .filter(|(key, value)| key.0 == terrain && value.1 == self.frame)
            .map(|(_, value)| value.2)
            .sum()
    }

    /// End the frame: every live batch not needed this frame (out of reach, a terrain gone or back on the
    /// per-instance path) is removed.
    pub(crate) fn end_frame(&mut self) {
        let frame = self.frame;
        let remove = &mut self.changes.remove;
        self.live.retain(|&(terrain, cell, kind), &mut (_, last, _)| {
            let keep = last == frame;
            if !keep {
                remove.push(batch_id(terrain, cell, kind));
            }
            keep
        });
        let seen = &self.seen;
        self.cells.retain(|terrain, _| seen.contains(terrain));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn piece(index_count: u32, is_leaf: bool) -> Piece {
        Piece { vertex_offset: index_count, index_offset: 0, index_count, material_id: 0, local: Mat4::IDENTITY, is_leaf }
    }

    fn distances(cull: f32, shadow: f32) -> KindDistances {
        KindDistances {
            cull_sq: cull * cull,
            shadow_sq: shadow * shadow,
            lod_sq: f32::INFINITY,
            legacy_lod: 0.0,
            legacy_impostor: 0.0,
        }
    }

    #[test]
    fn a_kind_draws_to_its_reach_and_casts_to_its_shadow_distance() {
        let parts = kind_parts(130, &[piece(300, false)], None, distances(60.0, 20.0));
        assert_eq!((parts[0].near, parts[0].far, parts[0].shadow_far, parts[0].casts_shadow), (0.0, 60.0, 20.0, true));
        let unlimited = KindDistances { cull_sq: f32::MAX, shadow_sq: f32::MAX, ..distances(0.0, 0.0) };
        let parts = kind_parts(130, &[piece(300, false)], None, unlimited);
        assert_eq!((parts[0].far, parts[0].shadow_far, parts[0].casts_shadow), (0.0, 0.0, true));
        let parts = kind_parts(130, &[piece(300, false)], None, distances(60.0, 0.0));
        assert!(!parts[0].casts_shadow, "a shadow distance of 0 casts nothing");
    }

    #[test]
    fn a_project_lod_mesh_takes_over_at_its_distance() {
        let d = KindDistances { lod_sq: 150.0 * 150.0, ..distances(400.0, 200.0) };
        let parts = kind_parts(140, &[piece(9000, false)], Some(&[piece(800, false)]), d);
        let bands: Vec<_> = parts.iter().map(|p| (p.index_count, p.near, p.far)).collect();
        assert_eq!(bands, [(9000, 0.0, 150.0), (800, 150.0, 400.0)]);
    }

    #[test]
    fn legacy_trees_drop_leaves_then_keep_only_bark() {
        let d = KindDistances { legacy_lod: 45.0, legacy_impostor: 90.0, ..distances(300.0, 100.0) };
        let tree = [piece(5000, false), piece(20000, true), piece(2000, false)];
        let fars: Vec<f32> = kind_parts(5, &tree, None, d).iter().map(|p| p.far).collect();
        // Bark to the reach; leaves gone past lod_distance.
        assert_eq!(fars, [300.0, 45.0, 300.0]);
        // No leaf marked: the heaviest goes past lod_distance, all but the cheapest past impostor_distance.
        let plain = [piece(5000, false), piece(20000, false), piece(2000, false)];
        let fars: Vec<f32> = kind_parts(5, &plain, None, d).iter().map(|p| p.far).collect();
        assert_eq!(fars, [90.0, 45.0, 300.0]);
        // Project kinds (128 and up) keep every part.
        let fars: Vec<f32> = kind_parts(200, &plain, None, d).iter().map(|p| p.far).collect();
        assert_eq!(fars, [300.0; 3]);
    }

    fn field() -> Vec<PaintedFoliage> {
        (0..200)
            .map(|i| PaintedFoliage {
                kind: 130 + (i % 2) as u8,
                position: glam::Vec3::new((i % 20) as f32 * 20.0, 0.0, (i / 20) as f32 * 20.0),
                yaw: 0.0,
                tilt: 0.0,
                scale: 1.0,
            })
            .collect()
    }

    fn parts_for(reach_a: f32, reach_b: f32) -> [Option<Vec<BatchPart>>; 256] {
        let mut parts: [Option<Vec<BatchPart>>; 256] = std::array::from_fn(|_| None);
        parts[130] = Some(kind_parts(130, &[piece(30, false)], None, distances(reach_a, 0.0)));
        parts[131] = Some(kind_parts(131, &[piece(60, false)], None, distances(reach_b, 0.0)));
        parts
    }

    #[test]
    fn cells_are_set_as_they_come_into_reach_and_removed_past_it() {
        let instances = field();
        let mut batches = FoliageBatches::default();
        let parts = parts_for(50.0, 150.0);
        batches.begin_frame();
        let copies = batches.terrain(1, 1, &instances, Mat4::IDENTITY, glam::Vec3::new(0.0, 0.0, 0.0), &parts);
        batches.end_frame();
        let first = batches.changes.set.len();
        assert!(first > 0 && copies > 0);
        let wide = batches.changes.set.iter().filter(|(id, _)| id & 0xFF == 131).count();
        let short = batches.changes.set.iter().filter(|(id, _)| id & 0xFF == 130).count();
        assert!(wide > short, "the longer-reach kind is set over more cells");
        // The same frame again: nothing to send.
        batches.begin_frame();
        batches.terrain(1, 1, &instances, Mat4::IDENTITY, glam::Vec3::ZERO, &parts);
        batches.end_frame();
        assert!(batches.changes.set.is_empty() && batches.changes.remove.is_empty());
        // Far away: everything removed.
        batches.begin_frame();
        batches.terrain(1, 1, &instances, Mat4::IDENTITY, glam::Vec3::new(5000.0, 0.0, 5000.0), &parts);
        batches.end_frame();
        assert_eq!(batches.changes.remove.len(), first);
        // A repaint (new revision) rebuilds what is in reach; a terrain no longer seen is removed.
        batches.begin_frame();
        batches.terrain(1, 2, &instances, Mat4::IDENTITY, glam::Vec3::ZERO, &parts);
        batches.end_frame();
        assert_eq!(batches.changes.set.len(), first);
        batches.begin_frame();
        batches.end_frame();
        assert_eq!(batches.changes.remove.len(), first);
    }

    #[test]
    fn a_cell_just_past_reach_is_kept_until_it_is_well_past() {
        let instances = field();
        let mut batches = FoliageBatches::default();
        let parts = parts_for(50.0, 50.0);
        batches.begin_frame();
        batches.terrain(1, 1, &instances, Mat4::IDENTITY, glam::Vec3::ZERO, &parts);
        batches.end_frame();
        // 20 m on: cells that left the 50 m reach are within the 32 m it is kept for.
        batches.begin_frame();
        batches.terrain(1, 1, &instances, Mat4::IDENTITY, glam::Vec3::new(20.0, 0.0, 0.0), &parts);
        batches.end_frame();
        assert!(batches.changes.remove.is_empty());
    }
}
