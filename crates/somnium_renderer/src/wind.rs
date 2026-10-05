//! Foliage wind: the per-frame uniform and a CPU mirror of `wind.wgsl`.
//!
//! Plants sway by displacing their vertices in world space, in every pass that
//! positions geometry — the visibility raster, the shadow cascades, the shading
//! pass's triangle reconstruction and the velocity pass — so the pixel that is
//! shaded, the shadow it casts and the motion vector TAA and motion blur read
//! all agree. Nothing is simulated and nothing is written per entity: the whole
//! of the state is one `vec4` in the view buffer and two numbers on each
//! material (glTF extras `somnium_wind_bend`, `somnium_wind_flutter`).
//!
//! The displacement is a function of the vertex's height above its instance's
//! pivot (the plant's base), so a trunk and the needles on it bend together
//! when both materials carry the same `bend`:
//!
//! - **bend** is a cantilever lean, `bend * h²` metres, downwind, with a slow
//!   sway round it and gust fronts that travel across the stand at the wind's
//!   heading. A pine leans a few tens of centimetres at its top; a fern a few
//!   centimetres at its tips.
//! - **flutter** is fast, small motion decorrelated by world position: needles,
//!   leaves and grass blades, never the trunk.
//!
//! Ray-traced acceleration structures keep the rest pose; the amplitude is
//! small and fades out by [`FoliageWind::fade_distance`], so traced shadows and
//! GI do not shimmer against the raster.
//!
//! This file is the CPU twin of `shaders/wind.wgsl` and the tests pin the
//! properties both must have. Change them together.

use glam::{Vec2, Vec3};

/// What `set_foliage_wind` stores: the scene's one wind (from `somnium.Weather`)
/// and how strongly plants answer it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FoliageWind {
    /// Wind velocity over the ground, metres per second (x, z).
    pub vector: [f32; 2],
    /// Authored sway multiplier (`Weather.foliage_sway`) times the graphics
    /// preset's gate. Zero stills every plant.
    pub strength: f32,
    /// Beyond this horizontal distance from the camera plants are still; the
    /// last third of it fades.
    pub fade_distance: f32,
}

impl Default for FoliageWind {
    fn default() -> Self {
        Self {
            vector: [0.0, 0.0],
            strength: 0.0,
            fade_distance: DEFAULT_FADE_DISTANCE,
        }
    }
}

/// Metres. Only the plants round the player move: past 30 m a lean of a few
/// centimetres still crawls at 1440p, and in a dense wood it read as the whole
/// far stand wobbling.
pub const DEFAULT_FADE_DISTANCE: f32 = 30.0;

impl FoliageWind {
    /// The `wind` vec4 as the shaders read it.
    #[must_use]
    pub fn uniform(&self) -> [f32; 4] {
        [
            self.vector[0],
            self.vector[1],
            self.strength.max(0.0),
            self.fade_distance.max(1.0),
        ]
    }
}

fn smoothstep(e0: f32, e1: f32, x: f32) -> f32 {
    let t = ((x - e0) / (e1 - e0)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

fn fract(x: f32) -> f32 {
    x - x.floor()
}

/// Displacement of a vertex at world `p` on a plant standing at `pivot`.
/// Mirrors `wind_offset` in `wind.wgsl` term for term.
#[must_use]
pub fn offset(
    p: Vec3,
    pivot: Vec3,
    bend: f32,
    flutter: f32,
    time: f32,
    wind: [f32; 4],
    camera: Vec3,
) -> Vec3 {
    let velocity = Vec2::new(wind[0], wind[1]);
    let speed = velocity.length();
    if (bend <= 0.0 && flutter <= 0.0) || speed < 0.01 || wind[2] <= 0.0 {
        return Vec3::ZERO;
    }
    let fade = 1.0
        - smoothstep(
            wind[3] * 0.65,
            wind[3],
            Vec2::new(p.x - camera.x, p.z - camera.z).length(),
        );
    if fade <= 0.0 {
        return Vec3::ZERO;
    }
    let dir = velocity / speed;
    let h = (p.y - pivot.y).max(0.0);
    let phase = fract((pivot.x * 12.9898 + pivot.z * 78.233).sin() * 43758.547) * std::f32::consts::TAU;
    // Gust fronts travel downwind across the stand at about 10 m/s.
    let front = (pivot.x * dir.x + pivot.z * dir.y) * 0.09 - time * 0.9;
    let gust = 0.55 + 0.45 * (0.5 + 0.5 * front.sin()) * (0.6 + 0.4 * (time * 0.37 + phase).sin());
    // Drag grows with speed and saturates: a gale does not lay a pine flat.
    let drag = (speed * 0.22).min(1.6);
    let sway = 0.7 + 0.3 * (time * 0.9 + phase).sin();
    let lean = bend * h * h * drag * gust * sway;
    let mut out = Vec3::new(dir.x * lean, 0.0, dir.y * lean);
    // A bent stem keeps its length, so its tip drops as it leans.
    out.y = -0.5 * lean * lean / h.max(0.25);
    let q = p * 1.7;
    let f = flutter * drag * (h * 2.0).clamp(0.0, 1.0) * (0.5 + 0.5 * gust);
    out += Vec3::new(
        (time * 5.3 + q.x + q.y * 0.7 + phase).sin(),
        0.6 * (time * 6.7 + q.z * 1.3 + q.x * 0.5).sin(),
        (time * 4.9 + q.y + q.z).sin(),
    ) * f;
    out * (fade * wind[2])
}

#[cfg(test)]
mod tests {
    use super::*;

    const BREEZE: [f32; 4] = [0.0, 4.0, 1.0, DEFAULT_FADE_DISTANCE];

    fn at(h: f32, bend: f32, flutter: f32, t: f32) -> Vec3 {
        offset(Vec3::new(3.0, h, 2.0), Vec3::new(3.0, 0.0, 2.0), bend, flutter, t, BREEZE, Vec3::ZERO)
    }

    #[test]
    fn the_base_of_a_plant_never_moves() {
        for t in [0.0, 1.3, 7.9] {
            assert!(at(0.0, 0.0004, 0.03, t).length() < 1e-6);
        }
    }

    #[test]
    fn a_tree_leans_downwind_more_the_higher_you_look() {
        let low = at(5.0, 0.00035, 0.0, 2.0);
        let top = at(28.0, 0.00035, 0.0, 2.0);
        assert!(top.z > low.z && low.z > 0.0, "{low} {top}");
        // A mature pine in a breeze: tens of centimetres, not metres.
        assert!(top.z > 0.04 && top.z < 0.6, "{top}");
    }

    #[test]
    fn it_moves_over_time() {
        let a = at(20.0, 0.00035, 0.02, 1.0);
        let b = at(20.0, 0.00035, 0.02, 1.5);
        assert!((a - b).length() > 1e-3);
    }

    #[test]
    fn no_wind_no_strength_or_no_authored_response_is_still() {
        let still = [0.0, 0.0, 1.0, DEFAULT_FADE_DISTANCE];
        let off = [0.0, 4.0, 0.0, DEFAULT_FADE_DISTANCE];
        let p = Vec3::new(0.0, 10.0, 0.0);
        assert_eq!(offset(p, Vec3::ZERO, 0.001, 0.02, 3.0, still, Vec3::ZERO), Vec3::ZERO);
        assert_eq!(offset(p, Vec3::ZERO, 0.001, 0.02, 3.0, off, Vec3::ZERO), Vec3::ZERO);
        assert_eq!(offset(p, Vec3::ZERO, 0.0, 0.0, 3.0, BREEZE, Vec3::ZERO), Vec3::ZERO);
    }

    #[test]
    fn plants_beyond_the_fade_distance_are_still() {
        let far = Vec3::new(0.0, 0.0, DEFAULT_FADE_DISTANCE + 1.0);
        let p = far + Vec3::Y * 10.0;
        assert_eq!(offset(p, far, 0.001, 0.02, 3.0, BREEZE, Vec3::ZERO), Vec3::ZERO);
    }

    #[test]
    fn the_uniform_is_what_the_shader_reads() {
        let w = FoliageWind {
            vector: [1.5, -2.0],
            strength: 0.8,
            fade_distance: 40.0,
        };
        assert_eq!(w.uniform(), [1.5, -2.0, 0.8, 40.0]);
        assert_eq!(FoliageWind::default().uniform()[2], 0.0);
    }
}
