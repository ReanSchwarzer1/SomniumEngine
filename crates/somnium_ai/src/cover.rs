//! Cover for ranged agents: spots near a body judged against its threats with
//! the caller's own sight test, so walls, pillars and ledges all count.
//! Spots come from rings around the body, snapped to the navmesh when there
//! is one (so each has room to stand).
//!
//! ```text
//! candidates(around, rings, nav) ──▶ judge(from, spots, stance, threats, blocked)
//!                                        └─▶ cover(&judged, threats)   nearest hidden from all
//!                                        └─▶ firing(&judged)           nearest with a clear shot
//! ```
use crate::navigation::NavWorld;
use glam::Vec3;

/// Where on a body sight lines start and end, metres above the feet.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Stance {
    /// Eye height.
    pub eye: f32,
    /// Chest height (what a shot aims at).
    pub chest: f32,
}

impl Default for Stance {
    fn default() -> Self {
        Self {
            eye: 1.6,
            chest: 1.1,
        }
    }
}

/// Someone to hide from or shoot at.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Threat {
    /// Feet.
    pub feet: Vec3,
    /// Its eye and chest.
    pub stance: Stance,
}

/// A spot judged against threats.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Spot {
    /// Feet there.
    pub point: Vec3,
    /// Threats that cannot see a body standing there (eye to chest blocked).
    pub hidden: usize,
    /// Threats a body there could shoot (its eye to their chest clear).
    pub sees: usize,
    /// Straight-line distance from where the body stands now.
    pub distance: f32,
}

/// `per_ring` spots on each of `rings` (metres) around `around`, nearest
/// ring first. With a navmesh each is moved to the mesh within a metre, and
/// dropped when there is none.
pub fn candidates(
    around: Vec3,
    rings: &[f32],
    per_ring: usize,
    nav: Option<&NavWorld>,
) -> Vec<Vec3> {
    rings
        .iter()
        .flat_map(|&ring| {
            (0..per_ring).map(move |k| {
                let a = k as f32 * std::f32::consts::TAU / per_ring.max(1) as f32;
                around + Vec3::new(a.cos(), 0.0, a.sin()) * ring
            })
        })
        .filter_map(|p| match nav {
            Some(nav) => nav.nearest(p, 1.0),
            None => Some(p),
        })
        .collect()
}

/// Judge each spot against `threats` for a body of `stance` now at `from`,
/// with `blocked(from, to)` the caller's sight line test (its physics).
pub fn judge(
    from: Vec3,
    spots: &[Vec3],
    stance: Stance,
    threats: &[Threat],
    blocked: impl Fn(Vec3, Vec3) -> bool,
) -> Vec<Spot> {
    spots
        .iter()
        .map(|&point| {
            let (eye, chest) = (point + Vec3::Y * stance.eye, point + Vec3::Y * stance.chest);
            let hidden = threats
                .iter()
                .filter(|t| blocked(t.feet + Vec3::Y * t.stance.eye, chest))
                .count();
            let sees = threats
                .iter()
                .filter(|t| !blocked(eye, t.feet + Vec3::Y * t.stance.chest))
                .count();
            Spot {
                point,
                hidden,
                sees,
                distance: point.distance(from),
            }
        })
        .collect()
}

/// The nearest spot hidden from all `threats` (the count judged against).
pub fn cover(spots: &[Spot], threats: usize) -> Option<Spot> {
    spots
        .iter()
        .filter(|s| threats > 0 && s.hidden == threats)
        .min_by(|a, b| a.distance.total_cmp(&b.distance))
        .copied()
}

/// The nearest spot with a clear shot at a threat.
pub fn firing(spots: &[Spot]) -> Option<Spot> {
    spots
        .iter()
        .filter(|s| s.sees > 0)
        .min_by(|a, b| a.distance.total_cmp(&b.distance))
        .copied()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Whether the segment crosses an axis-aligned box (slab test).
    fn through(min: Vec3, max: Vec3) -> impl Fn(Vec3, Vec3) -> bool {
        move |a: Vec3, b: Vec3| {
            let d = b - a;
            let (mut lo, mut hi) = (0.0f32, 1.0f32);
            for i in 0..3 {
                if d[i].abs() < 1e-6 {
                    if a[i] < min[i] || a[i] > max[i] {
                        return false;
                    }
                    continue;
                }
                let (t0, t1) = ((min[i] - a[i]) / d[i], (max[i] - a[i]) / d[i]);
                lo = lo.max(t0.min(t1));
                hi = hi.min(t0.max(t1));
                if lo > hi {
                    return false;
                }
            }
            true
        }
    }

    #[test]
    fn the_nearest_spot_behind_a_pillar_hides_and_one_beside_it_shoots() {
        // A rifleman 15 m from its threat, a pillar a little to its right.
        let me = Vec3::new(0.0, 0.0, -15.0);
        let threat = Threat {
            feet: Vec3::ZERO,
            stance: Stance::default(),
        };
        let pillar = through(Vec3::new(1.9, 0.0, -13.6), Vec3::new(3.1, 3.0, -12.4));
        let spots = candidates(me, &[1.5, 3.0, 4.5], 8, None);
        assert_eq!(spots.len(), 24);
        let judged = judge(me, &spots, Stance::default(), &[threat], &pillar);
        let hide = cover(&judged, 1).expect("a spot behind the pillar");
        assert!(pillar(
            threat.feet + Vec3::Y * 1.6,
            hide.point + Vec3::Y * 1.1
        ));
        assert!(hide.distance <= 3.01, "the nearest: {hide:?}");
        // From the hiding spot, the nearest clear shot is a step aside.
        let again = judge(hide.point, &spots, Stance::default(), &[threat], &pillar);
        let shot = firing(&again).expect("a spot with a shot");
        assert!(!pillar(shot.point + Vec3::Y * 1.6, Vec3::Y * 1.1));
        assert!(
            cover(&judged, 0).is_none(),
            "no threats, nothing to hide from"
        );
    }

    #[test]
    fn spots_snap_to_the_navmesh_or_are_dropped() {
        use crate::navigation::{BakeInput, BakeSettings, Bounds, bake};
        let floor = vec![
            [[0.0, 0.0, 0.0], [10.0, 0.0, 0.0], [10.0, 0.0, 10.0]],
            [[0.0, 0.0, 0.0], [10.0, 0.0, 10.0], [0.0, 0.0, 10.0]],
        ];
        let tile = bake(&BakeInput {
            cell: [0, 0, 0],
            bounds: Bounds {
                min: [0.0, -1.0, 0.0],
                max: [10.0, 4.0, 10.0],
            },
            triangles: floor,
            obstacles: Vec::new(),
            settings: BakeSettings {
                voxel_size: 0.5,
                agent_radius: 0.0,
                ..BakeSettings::default()
            },
        })
        .unwrap();
        let mut nav = NavWorld::default();
        nav.install(tile);
        // At the floor's corner, rings reach off it: some snap back, the
        // farthest are dropped.
        let spots = candidates(Vec3::new(0.5, 0.0, 0.5), &[1.0, 4.0], 8, Some(&nav));
        assert!(!spots.is_empty() && spots.len() < 16, "{}", spots.len());
        assert!(
            spots
                .iter()
                .all(|p| (0.0..=10.0).contains(&p.x) && (0.0..=10.0).contains(&p.z))
        );
    }
}
