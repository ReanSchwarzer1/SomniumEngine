//! Per-cascade cache policy for the conventional shadow atlas.
//!
//! The atlas is persistent already; this module decides which of its four
//! quadrants must be redrawn.  It deliberately owns no `wgpu` values, so the
//! invalidation contract can be tested without a device:
//!
//! - a never-rendered cascade is always drawn;
//! - nearby cascades update immediately when the light or snapped camera cell
//!   changes;
//! - at most one already-valid distant cascade (2/3) takes a view update per
//!   frame;
//! - caster changes invalidate only the cascades whose volumes contain them;
//! - disabling the cache redraws all four and restores the old behaviour.

use glam::Vec3;

use super::{NUM_CASCADES, cascade::CascadeData};

/// Increasing angular tolerance for increasingly distant cascades.
///
/// Values are radians.  The nearest tier moves after about 0.006 degrees; the
/// last tolerates about 0.046 degrees before taking a new light projection.
const LIGHT_EPSILON: [f32; NUM_CASCADES] = [0.000_1, 0.000_2, 0.000_4, 0.000_8];

#[derive(Clone, Copy, Debug)]
struct Entry {
    cascade: CascadeData,
    light_direction: Vec3,
    snapped_cell: [i64; 3],
    caster_revision: Option<u64>,
    valid: bool,
}

impl Default for Entry {
    fn default() -> Self {
        Self {
            cascade: CascadeData::default(),
            light_direction: Vec3::Y,
            snapped_cell: [0; 3],
            caster_revision: None,
            valid: false,
        }
    }
}

/// First half of one cache update.
///
/// `cascades` are the matrices that both culling and shading must use this
/// frame.  A staggered distant update keeps the previous matrix here until its
/// quadrant is actually redrawn; sampling a new matrix from old depth would be
/// a correctness bug.
#[derive(Clone, Copy, Debug)]
pub struct CascadeCacheFrame {
    pub cascades: [CascadeData; NUM_CASCADES],
    pub view_dirty: [bool; NUM_CASCADES],
}

/// Persistent policy state for the four atlas quadrants.
pub struct CascadeShadowCache {
    entries: [Entry; NUM_CASCADES],
    next_distant: usize,
}

impl Default for CascadeShadowCache {
    fn default() -> Self {
        Self {
            entries: [Entry::default(); NUM_CASCADES],
            next_distant: 2,
        }
    }
}

impl CascadeShadowCache {
    /// Resolve camera/light invalidation before any caster culling is built.
    pub fn begin_frame(
        &mut self,
        candidates: [CascadeData; NUM_CASCADES],
        light_direction: Vec3,
        enabled: bool,
    ) -> CascadeCacheFrame {
        let light_direction = light_direction.normalize_or(Vec3::Y);
        if !enabled {
            for (i, entry) in self.entries.iter_mut().enumerate() {
                install_view(entry, candidates[i], light_direction);
            }
            return CascadeCacheFrame {
                cascades: candidates,
                view_dirty: [true; NUM_CASCADES],
            };
        }

        let requested = std::array::from_fn(|i| {
            view_needs_update(&self.entries[i], candidates[i], light_direction, i)
        });
        let mut selected = requested;

        // Never stagger first use: an uninitialised quadrant cannot be sampled.
        // Once both distant entries are valid, update only one when both ask in
        // the same frame.  The other keeps its old matrix and is selected next
        // frame because the request remains true.
        if self.entries[2].valid && self.entries[3].valid && requested[2] && requested[3] {
            let keep = self.next_distant;
            selected[if keep == 2 { 3 } else { 2 }] = false;
            self.next_distant = if keep == 2 { 3 } else { 2 };
        }

        for i in 0..NUM_CASCADES {
            if selected[i] {
                install_view(&mut self.entries[i], candidates[i], light_direction);
            }
        }

        CascadeCacheFrame {
            cascades: self.entries.map(|entry| entry.cascade),
            view_dirty: selected,
        }
    }

    /// Add caster invalidation after the frame's filtered caster set exists.
    ///
    /// The returned mask is the render-pass contract: false quadrants are left
    /// untouched in the persistent atlas.
    pub fn finish_frame(
        &mut self,
        caster_revisions: [u64; NUM_CASCADES],
        view_dirty: [bool; NUM_CASCADES],
        enabled: bool,
    ) -> [bool; NUM_CASCADES] {
        if !enabled {
            for (entry, revision) in self.entries.iter_mut().zip(caster_revisions) {
                entry.caster_revision = Some(revision);
            }
            return [true; NUM_CASCADES];
        }

        std::array::from_fn(|i| {
            let caster_dirty = self.entries[i].caster_revision != Some(caster_revisions[i]);
            let dirty = view_dirty[i] || caster_dirty;
            if dirty {
                self.entries[i].caster_revision = Some(caster_revisions[i]);
            }
            dirty
        })
    }
}

fn install_view(entry: &mut Entry, cascade: CascadeData, light_direction: Vec3) {
    entry.cascade = cascade;
    entry.light_direction = light_direction;
    entry.snapped_cell = snapped_cell(cascade.world_center, light_direction, cascade.texel_size);
    entry.valid = true;
}

fn view_needs_update(
    entry: &Entry,
    candidate: CascadeData,
    light_direction: Vec3,
    cascade: usize,
) -> bool {
    if !entry.valid {
        return true;
    }
    let angular_change = entry
        .light_direction
        .dot(light_direction)
        .clamp(-1.0, 1.0)
        .acos();
    if angular_change > LIGHT_EPSILON[cascade] {
        return true;
    }

    // A projection-size change alters the world metres represented by a texel,
    // even if its centre stayed put.  Perspective/FOV changes must therefore
    // redraw rather than reinterpreting old depth under a new projection.
    let texel_tolerance = entry.cascade.texel_size.abs().max(1.0e-6) * 1.0e-4;
    if (candidate.texel_size - entry.cascade.texel_size).abs() > texel_tolerance {
        return true;
    }

    snapped_cell(
        candidate.world_center,
        entry.light_direction,
        entry.cascade.texel_size,
    ) != entry.snapped_cell
}

fn snapped_cell(center: Vec3, light_direction: Vec3, texel_size: f32) -> [i64; 3] {
    let light = light_direction.normalize_or(Vec3::Y);
    let up_hint = if light.dot(Vec3::Y).abs() > 0.99 {
        Vec3::Z
    } else {
        Vec3::Y
    };
    let right = light.cross(up_hint).normalize_or(Vec3::X);
    let up = right.cross(light).normalize_or(Vec3::Y);
    let step = texel_size.abs().max(1.0e-6);
    [right, up, light].map(|axis| (center.dot(axis) / step).floor() as i64)
}

#[cfg(test)]
mod tests {
    use glam::{Mat4, Vec3};

    use super::*;

    fn candidates(offset: Vec3) -> [CascadeData; NUM_CASCADES] {
        std::array::from_fn(|i| CascadeData {
            view_proj: Mat4::from_translation(-offset),
            split_depth: (i + 1) as f32 * 25.0,
            world_center: offset,
            texel_size: (i + 1) as f32,
        })
    }

    #[test]
    fn first_frame_renders_every_quadrant_then_a_static_frame_renders_none() {
        let mut cache = CascadeShadowCache::default();
        let first = cache.begin_frame(candidates(Vec3::ZERO), Vec3::Y, true);
        assert_eq!(first.view_dirty, [true; 4]);
        assert_eq!(
            cache.finish_frame([7; 4], first.view_dirty, true),
            [true; 4]
        );

        let second = cache.begin_frame(candidates(Vec3::ZERO), Vec3::Y, true);
        assert_eq!(second.view_dirty, [false; 4]);
        assert_eq!(
            cache.finish_frame([7; 4], second.view_dirty, true),
            [false; 4]
        );
    }

    #[test]
    fn a_caster_revision_only_invalidates_its_cascade() {
        let mut cache = CascadeShadowCache::default();
        let first = cache.begin_frame(candidates(Vec3::ZERO), Vec3::Y, true);
        cache.finish_frame([10, 20, 30, 40], first.view_dirty, true);

        let frame = cache.begin_frame(candidates(Vec3::ZERO), Vec3::Y, true);
        assert_eq!(
            cache.finish_frame([10, 21, 30, 40], frame.view_dirty, true),
            [false, true, false, false]
        );
    }

    #[test]
    fn movement_inside_one_shadow_texel_keeps_the_cached_matrix() {
        let mut cache = CascadeShadowCache::default();
        let first = cache.begin_frame(candidates(Vec3::splat(0.25)), Vec3::Y, true);
        cache.finish_frame([1; 4], first.view_dirty, true);

        let mut moved = candidates(Vec3::splat(0.40));
        // Keep every centre inside its previous floor-divided cell.
        for (i, c) in moved.iter_mut().enumerate() {
            c.texel_size = (i + 1) as f32;
        }
        let frame = cache.begin_frame(moved, Vec3::Y, true);
        assert_eq!(frame.view_dirty, [false; 4]);
    }

    #[test]
    fn simultaneous_distant_view_changes_are_interleaved() {
        let mut cache = CascadeShadowCache::default();
        let first = cache.begin_frame(candidates(Vec3::ZERO), Vec3::Y, true);
        cache.finish_frame([1; 4], first.view_dirty, true);

        let moved = candidates(Vec3::splat(10.0));
        let a = cache.begin_frame(moved, Vec3::Y, true);
        assert!(a.view_dirty[0] && a.view_dirty[1]);
        assert_ne!(a.view_dirty[2], a.view_dirty[3]);
        cache.finish_frame([1; 4], a.view_dirty, true);

        let b = cache.begin_frame(moved, Vec3::Y, true);
        assert!(!b.view_dirty[0] && !b.view_dirty[1]);
        assert_ne!(b.view_dirty[2], b.view_dirty[3]);
        assert_ne!(a.view_dirty[2], b.view_dirty[2]);
    }

    #[test]
    fn kill_switch_restores_four_cascades_every_frame() {
        let mut cache = CascadeShadowCache::default();
        for _ in 0..2 {
            let frame = cache.begin_frame(candidates(Vec3::ZERO), Vec3::Y, false);
            assert_eq!(frame.view_dirty, [true; 4]);
            assert_eq!(
                cache.finish_frame([0; 4], frame.view_dirty, false),
                [true; 4]
            );
        }
    }
}
