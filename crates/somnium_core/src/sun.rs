//! Sunlight reaching the ground, as a function of the sun's altitude
//! (Phase 25M).
//!
//! # The bug this exists for
//!
//! `LightComponent::photometric_color` returns intensity × tint and nothing
//! else. A directional light authored at 100 000 lux stayed at 100 000 lux when
//! the gizmo rotated it below the horizon, so the engine went on lighting the
//! world with full noon sunlight arriving from underground. The atmosphere
//! shaders were doing their job — they guard the sun's own transmittance with
//! `ray_hits_ground` — but nothing told the *direct* light that the sun had set.
//!
//! # What this computes
//!
//! The transmittance of the atmosphere along the ray from the ground toward the
//! sun: how much of the sun's light survives the trip. It is the same quantity
//! the transmittance LUT holds, evaluated on the CPU for the single direction
//! that matters, which buys two things a shader-side fix would not:
//!
//! - **One place.** Everything downstream — direct shading, shadows, ReSTIR DI
//!   and GI, the froxel volume, and the sky's own `sun_illuminance` and its
//!   moon/star blending — reads the light buffer. Attenuating there fixes all of
//!   them at once and cannot drift between them.
//! - **Colour, not just brightness.** Rayleigh extinction removes blue first, so
//!   a low sun comes out orange on its own, from physics rather than from an
//!   authored gradient. That is the whole reason to integrate rather than to
//!   multiply by a fade curve.
//!
//! The constants mirror `shaders/atmosphere.wgsl` exactly; they are Hillaire's
//! (2020) parameters for Earth, in kilometres and km⁻¹.

use glam::Vec3;

/// Planet radius, km. Mirrors `GROUND_RADIUS` in `atmosphere.wgsl`.
const GROUND_RADIUS: f32 = 6360.0;
/// Top of the atmosphere, km. Mirrors `ATMOS_RADIUS`.
const ATMOS_RADIUS: f32 = 6460.0;

const RAYLEIGH_SCATTERING: Vec3 = Vec3::new(5.802e-3, 13.558e-3, 33.1e-3);
const RAYLEIGH_SCALE_H: f32 = 8.0;
const MIE_EXTINCTION: f32 = 4.4e-3;
const MIE_SCALE_H: f32 = 1.2;
const OZONE_ABSORPTION: Vec3 = Vec3::new(0.650e-3, 1.881e-3, 0.085e-3);
const OZONE_CENTER: f32 = 25.0;
const OZONE_WIDTH: f32 = 15.0;

/// Steps along the sun ray. The integrand is smooth and the ray is at most a
/// few hundred kilometres, so this converges quickly; 32 is what the LUT
/// generator uses.
const STEPS: u32 = 32;

/// Angle below the horizon at which the sun is fully gone, in radians.
///
/// Not zero. The sun's disc is about half a degree across and refraction lifts
/// it by roughly another half, so light keeps arriving after the geometric
/// centre has set. Cutting exactly at zero produces a visible step in the last
/// moment of sunset — the one moment anybody is watching.
const HORIZON_FADE: f32 = 0.015;

fn density(altitude: f32) -> Vec3 {
    Vec3::new(
        (-altitude / RAYLEIGH_SCALE_H).exp(),
        (-altitude / MIE_SCALE_H).exp(),
        (1.0 - (altitude - OZONE_CENTER).abs() / OZONE_WIDTH).max(0.0),
    )
}

fn extinction(altitude: f32) -> Vec3 {
    let d = density(altitude);
    RAYLEIGH_SCATTERING * d.x + Vec3::splat(MIE_EXTINCTION) * d.y + OZONE_ABSORPTION * d.z
}

/// Distance from radius `r` along a ray of cosine `mu` to the top of the
/// atmosphere.
fn distance_to_top(r: f32, mu: f32) -> f32 {
    let disc = r * r * (mu * mu - 1.0) + ATMOS_RADIUS * ATMOS_RADIUS;
    (-r * mu + disc.max(0.0).sqrt()).max(0.0)
}

/// True when a ray leaving radius `r` at cosine `mu` runs into the planet.
///
/// The same test `atmosphere.wgsl`'s `ray_hits_ground` makes, and the reason a
/// sun below the horizon must contribute nothing: its light would have to pass
/// through the Earth to arrive.
fn hits_ground(r: f32, mu: f32) -> bool {
    mu < 0.0 && r * r * (mu * mu - 1.0) + GROUND_RADIUS * GROUND_RADIUS >= 0.0
}

/// Fraction of sunlight, per channel, surviving the trip to an observer
/// `altitude_km` above the ground when the sun's direction has vertical
/// component `sun_up` (that is, `sin` of its elevation).
///
/// Returns zero once the sun is below the horizon.
#[must_use]
pub fn transmittance(sun_up: f32, altitude_km: f32) -> Vec3 {
    // Below the horizon there is no direct sun at all. The soft edge covers the
    // disc's own width and refraction; see `HORIZON_FADE`.
    if sun_up <= -HORIZON_FADE {
        return Vec3::ZERO;
    }
    let horizon = ((sun_up + HORIZON_FADE) / (2.0 * HORIZON_FADE)).clamp(0.0, 1.0);

    let r = GROUND_RADIUS + altitude_km.max(0.0);
    // Phase 25M-2: Clamp ray zenith cosine to a minimum elevation angle (~ -1.5° = -0.026 rad),
    // matching UE5's `TransmittanceMinLightElevationAngle`. This prevents the optical depth
    // integration from over-extinguishing into hyper-saturated red/zero right at the horizon.
    const MIN_ELEVATION_COS: f32 = -0.026;
    let mu = sun_up.max(MIN_ELEVATION_COS);
    if hits_ground(r, sun_up.max(0.0)) {
        return Vec3::ZERO;
    }

    // Optical depth along the ray, then Beer-Lambert.
    let t_max = distance_to_top(r, mu);
    let dt = t_max / STEPS as f32;
    let mut optical_depth = Vec3::ZERO;
    for i in 0..STEPS {
        let t = (i as f32 + 0.5) * dt;
        // Law of cosines: the radius at distance `t` along the ray.
        let ri = (r * r + t * t + 2.0 * r * t * mu).max(GROUND_RADIUS * GROUND_RADIUS).sqrt();
        optical_depth += extinction(ri - GROUND_RADIUS) * dt;
    }

    Vec3::new(
        (-optical_depth.x).exp(),
        (-optical_depth.y).exp(),
        (-optical_depth.z).exp(),
    ) * horizon
}

/// Direction toward the moon in world space, normalised.
///
/// Phase 25M-2: a simplified but physically-based model. The real moon orbits
/// the Earth every 29.53 days (synodic period) in a plane tilted ~5.14° from
/// the ecliptic. Rather than a full ephemeris, this uses:
///
/// - `time_days`: total engine time in fractional days (e.g. `engine_seconds / 86400`).
/// - `sun_direction`: normalised direction toward the sun.
///
/// The moon is placed at a `phase_angle` around the celestial sphere relative
/// to the sun direction, measured in the sun's orbital plane with a slight
/// tilt for the lunar inclination. At `phase_angle = π` the moon is opposite
/// the sun (full moon), at 0 it is aligned with the sun (new moon).
///
/// This gives a correct phase/position relationship: a crescent moon is near
/// the sun, a full moon is opposite, and a quarter moon is 90° away.
#[must_use]
pub fn moon_direction(sun_direction: Vec3, time_days: f64) -> Vec3 {
    use std::f64::consts::TAU;

    const SYNODIC_PERIOD: f64 = 29.530588; // days
    const INCLINATION: f64 = 5.14_f64; // degrees

    // Phase angle: how far around the orbit the moon is from the sun.
    // At t=0 we start at a full moon (phase_angle = π).
    let phase_frac = (time_days / SYNODIC_PERIOD).fract();
    let phase_angle = phase_frac * TAU;

    // Build a coordinate frame from the sun direction.
    // The sun direction defines one axis; we need two perpendicular axes.
    let sun = sun_direction.normalize_or(Vec3::Y);
    let up_hint = if sun.dot(Vec3::Y).abs() > 0.99 {
        Vec3::Z
    } else {
        Vec3::Y
    };
    let right = sun.cross(up_hint).normalize();
    let up = right.cross(sun).normalize();

    // Moon's position on the unit sphere: rotate around the sun direction
    // with a small tilt for lunar inclination.
    let incl_rad = (INCLINATION as f32).to_radians();
    let cos_phase = (phase_angle as f32).cos();
    let sin_phase = (phase_angle as f32).sin();

    // The moon lies on a great circle tilted by `incl_rad` from the
    // ecliptic plane defined by (sun, right). The `up` component gives
    // the out-of-plane tilt.
    let moon = -sun * cos_phase
        + right * sin_phase * incl_rad.cos()
        + up * sin_phase * incl_rad.sin();

    moon.normalize()
}


#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_sun_below_the_horizon_delivers_nothing() {
        // The bug in one line: this used to be full noon sunlight.
        assert_eq!(transmittance(-0.5, 0.0), Vec3::ZERO);
        assert_eq!(transmittance(-1.0, 0.0), Vec3::ZERO);
    }

    #[test]
    fn the_overhead_sun_is_barely_attenuated() {
        // Straight up through the whole atmosphere still loses a little blue,
        // but the sun at noon must not be dimmed to a candle.
        let t = transmittance(1.0, 0.0);
        assert!(t.x > 0.9, "red {}", t.x);
        assert!(t.z > 0.5, "blue {}", t.z);
        assert!(t.z < t.x, "blue should be extinguished more than red");
    }

    #[test]
    fn a_low_sun_comes_out_orange() {
        // The reason to integrate rather than fade: at a shallow angle the ray
        // is long, Rayleigh takes the blue out first, and sunset colour falls
        // out of the physics instead of an authored gradient.
        let noon = transmittance(1.0, 0.0);
        let low = transmittance(0.05, 0.0);
        let noon_ratio = noon.z / noon.x;
        let low_ratio = low.z / low.x;
        assert!(
            low_ratio < noon_ratio * 0.5,
            "low sun ratio {low_ratio} was not much redder than noon {noon_ratio}"
        );
    }

    #[test]
    fn brightness_falls_monotonically_as_the_sun_sets() {
        // A sunset must not brighten anywhere on the way down, or the sky will
        // flash as it crosses whatever the non-monotonic step is.
        let mut prev = f32::INFINITY;
        let mut mu = 1.0;
        while mu >= -0.05 {
            let lum = transmittance(mu, 0.0).length();
            assert!(lum <= prev + 1e-6, "brightened at sun_up {mu}: {lum} > {prev}");
            prev = lum;
            mu -= 0.01;
        }
    }

    #[test]
    fn the_horizon_crossing_is_continuous() {
        // Either side of the fade band the difference must be small: a step here
        // is a visible flash in the one moment somebody is watching.
        let a = transmittance(0.0, 0.0).length();
        let b = transmittance(-0.005, 0.0).length();
        assert!((a - b).abs() < 0.2, "step across the horizon: {a} vs {b}");
    }

    #[test]
    fn altitude_thins_the_air_above_you() {
        // At 10 km most of the atmosphere is below the observer, so more light
        // survives — the check that the altitude parameter is used at all.
        assert!(transmittance(0.2, 10.0).length() > transmittance(0.2, 0.0).length());
    }
}
