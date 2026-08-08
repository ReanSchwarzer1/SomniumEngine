//! Photometric light units and physical camera exposure (Phase 24A).
//!
//! Before this module every light carried a bare `intensity` multiplier with no
//! physical meaning. That is why turning the sun down could never produce night:
//! there was no scale to turn it down *on*, so "3.0" was neither noon nor dusk,
//! and the only way to change the mood of a scene was to fight the tone mapper.
//!
//! Everything here follows the photometric system the film and photography
//! industries already use, which is also what Filament and Bevy adopted:
//!
//! - **Directional lights** carry **illuminance** in lux (lm/m²). The sun is
//!   ~100 000 lux at noon and ~0.05 lux under a full moon — six orders of
//!   magnitude, which is exactly the range that makes exposure meaningful.
//! - **Point and spot lights** carry **luminous power** in lumens, the number
//!   printed on a real bulb's box, converted to candela for shading.
//! - **Cameras** carry aperture, shutter speed and ISO, combined into EV100.
//!
//! The payoff is that lighting a scene becomes a matter of looking up what the
//! real thing measures, rather than guessing multipliers until it looks right.

use std::f32::consts::PI;

/// Illuminance presets in **lux**, for directional lights and ambient levels.
///
/// Values follow standard photometric references. The span from `MOONLESS_NIGHT`
/// to `RAW_SUNLIGHT` is roughly 10⁹, which is why a physical camera model is not
/// optional once these are in use.
pub mod lux {
    /// Starlight only, no moon.
    pub const MOONLESS_NIGHT: f32 = 0.0001;
    /// Full moon on a clear night.
    pub const FULL_MOON: f32 = 0.05;
    /// Deep twilight, sun ~6° below the horizon.
    pub const CIVIL_TWILIGHT: f32 = 3.4;
    /// Typical domestic interior.
    pub const LIVING_ROOM: f32 = 50.0;
    /// Heavily overcast daylight.
    pub const DARK_OVERCAST_DAY: f32 = 100.0;
    /// Office lighting to code.
    pub const OFFICE: f32 = 320.0;
    /// Sun just above the horizon.
    pub const CLEAR_SUNRISE: f32 = 400.0;
    /// Overcast midday.
    pub const OVERCAST_DAY: f32 = 1_000.0;
    /// Daylight in shade — the usual default for an outdoor scene.
    pub const AMBIENT_DAYLIGHT: f32 = 10_000.0;
    /// Full daylight, sun not directly incident.
    pub const FULL_DAYLIGHT: f32 = 20_000.0;
    /// Direct midday sun.
    pub const DIRECT_SUNLIGHT: f32 = 100_000.0;
}

/// Luminous-power presets in **lumens**, for point and spot lights.
pub mod lumens {
    /// A candle flame.
    pub const CANDLE: f32 = 12.0;
    /// 40 W incandescent equivalent.
    pub const BULB_40W: f32 = 450.0;
    /// 60 W incandescent equivalent — the common household bulb.
    pub const BULB_60W: f32 = 800.0;
    /// 100 W incandescent equivalent.
    pub const BULB_100W: f32 = 1_600.0;
    /// A bright work light or small floodlight.
    pub const FLOODLIGHT: f32 = 20_000.0;
    /// Stadium-scale fixture.
    pub const VERY_LARGE_CINEMA_LIGHT: f32 = 1_000_000.0;
}

/// Convert a point light's luminous power (lumens) to luminous intensity
/// (candela, lm/sr).
///
/// A point light radiates into the full sphere, so its power is spread over 4π
/// steradians. Shading then divides by distance squared to reach illuminance.
#[must_use]
pub fn point_candela(power_lumens: f32) -> f32 {
    power_lumens / (4.0 * PI)
}

/// Convert a spot light's luminous power (lumens) to candela.
///
/// Deliberately uses the same 4π divisor as [`point_candela`] rather than
/// integrating over the actual cone. Dividing by the cone's true solid angle is
/// arguably more correct, but it means narrowing a spot makes it brighter — so
/// adjusting the cone silently changes the exposure of the scene, which is
/// miserable to work with. Filament makes the same trade for the same reason.
#[must_use]
pub fn spot_candela(power_lumens: f32, _outer_angle_rad: f32) -> f32 {
    power_lumens / (4.0 * PI)
}

/// Exposure value at ISO 100 for a set of physical camera settings.
///
/// `EV100 = log2(N² · 100 / (t · S))`, with aperture `N` in f-stops, shutter
/// time `t` in seconds and sensitivity `S` in ISO.
#[must_use]
pub fn ev100_from_camera(aperture_f_stops: f32, shutter_speed_s: f32, sensitivity_iso: f32) -> f32 {
    let denom = shutter_speed_s * sensitivity_iso;
    if denom <= 0.0 || aperture_f_stops <= 0.0 {
        return ev100::SUNLIGHT;
    }
    ((aperture_f_stops * aperture_f_stops * 100.0) / denom).log2()
}

/// Linear multiplier that converts scene luminance (cd/m²) to a ~[0, 1] range
/// before tone mapping.
///
/// The 1.2 is the standard reflected-light calibration constant, so a surface of
/// 18% reflectance under the metered illuminance lands at middle grey.
#[must_use]
pub fn exposure_from_ev100(ev100: f32) -> f32 {
    1.0 / (1.2 * ev100.exp2())
}

/// EV100 presets, for when the scene is set by eye rather than by camera.
pub mod ev100 {
    /// Bright sun. Pairs with [`super::lux::DIRECT_SUNLIGHT`].
    pub const SUNLIGHT: f32 = 15.0;
    /// Overcast exterior.
    pub const OVERCAST: f32 = 12.0;
    /// Interior, artificial light.
    pub const INDOOR: f32 = 7.0;
    /// Exterior at night under a full moon.
    pub const MOONLIT_NIGHT: f32 = -2.0;
}

/// Sky-dome luminance (cd/m²) per lux of sun illuminance.
///
/// A clear daytime sky measures roughly 8 000 cd/m² while the sun delivers about
/// 100 000 lux, so the ratio is ~0.08. This exists so the procedural sky scales
/// with the sun instead of staying a fixed brightness — the specific reason
/// lowering the sun currently cannot produce night.
///
/// **Interim.** Phase 24C replaces the whole gradient with real atmospheric
/// scattering, at which point sky luminance is computed rather than scaled and
/// this constant goes away.
pub const SKY_LUMINANCE_PER_LUX: f32 = 0.08;

#[cfg(test)]
mod tests {
    use super::*;

    /// The published EV100 for direct sun is 15, reached at f/16, 1/125 s,
    /// ISO 100. Note the shutter: the "sunny 16" rule of thumb says 1/ISO,
    /// which at ISO 100 is 1/100 s and actually lands on EV 14.64 — a third of
    /// a stop brighter. The rule is an approximation, EV15 is the definition.
    #[test]
    fn sunny_sixteen_lands_on_ev15() {
        let ev = ev100_from_camera(16.0, 1.0 / 125.0, 100.0);
        assert!((ev - 15.0).abs() < 0.05, "f/16 1/125 ISO100 gave EV100 {ev}, expected ~15");

        let rule_of_thumb = ev100_from_camera(16.0, 1.0 / 100.0, 100.0);
        assert!((rule_of_thumb - 14.64).abs() < 0.05, "sunny-16 rule gave {rule_of_thumb}");
    }

    /// Opening the aperture one stop (f/16 → f/11.3) halves the EV, and halving
    /// the shutter speed does the same. Exposure must respond to both.
    #[test]
    fn one_stop_changes_are_one_ev_apart() {
        let base = ev100_from_camera(16.0, 1.0 / 100.0, 100.0);
        let wider = ev100_from_camera(16.0 / core::f32::consts::SQRT_2, 1.0 / 100.0, 100.0);
        let slower = ev100_from_camera(16.0, 1.0 / 50.0, 100.0);
        assert!((base - wider - 1.0).abs() < 0.02);
        assert!((base - slower - 1.0).abs() < 0.02);
    }

    /// A white Lambertian surface in full sun should expose to near-white but
    /// not clip. This is the end-to-end check that lux, the 1/π Lambert factor
    /// and the exposure constant agree with each other.
    #[test]
    fn white_surface_in_sunlight_exposes_near_white() {
        let luminance = lux::DIRECT_SUNLIGHT / PI; // albedo 1, N·L 1
        let exposed = luminance * exposure_from_ev100(ev100::SUNLIGHT);
        assert!(
            (0.5..=1.0).contains(&exposed),
            "white in sun exposed to {exposed}, expected roughly 0.8",
        );
    }

    /// The same surface under a full moon, metered for a moonlit night, must
    /// also land in a visible range — otherwise night is just a black screen.
    #[test]
    fn moonlight_metered_for_night_is_visible() {
        let luminance = lux::FULL_MOON / PI;
        let exposed = luminance * exposure_from_ev100(ev100::MOONLIT_NIGHT);
        assert!(
            (0.01..=1.0).contains(&exposed),
            "moonlight exposed to {exposed}, expected something visible",
        );
    }

    /// ...but metered for daylight it must be essentially black. This is the
    /// property the whole phase exists to deliver.
    #[test]
    fn moonlight_metered_for_daylight_is_black() {
        let luminance = lux::FULL_MOON / PI;
        let exposed = luminance * exposure_from_ev100(ev100::SUNLIGHT);
        assert!(exposed < 0.001, "moonlight at daylight exposure gave {exposed}");
    }

    /// A 60 W-equivalent bulb is ~800 lm; at 1 m that should read as a sane
    /// interior illuminance rather than an arbitrary number.
    #[test]
    fn a_household_bulb_lights_a_room_plausibly() {
        let illuminance_at_1m = point_candela(lumens::BULB_60W); // E = I / d², d = 1
        assert!(
            (50.0..=100.0).contains(&illuminance_at_1m),
            "60 W bulb gave {illuminance_at_1m} lux at 1 m",
        );
    }

    /// Narrowing a spot must not change its brightness — see [`spot_candela`].
    #[test]
    fn spot_brightness_is_independent_of_cone_angle() {
        let narrow = spot_candela(lumens::BULB_100W, 0.1);
        let wide = spot_candela(lumens::BULB_100W, 1.2);
        assert_eq!(narrow, wide);
    }

    /// Degenerate camera settings must fall back rather than produce NaN or
    /// infinity, which would propagate into every pixel of the frame.
    #[test]
    fn degenerate_camera_settings_fall_back() {
        for ev in [
            ev100_from_camera(16.0, 0.0, 100.0),
            ev100_from_camera(16.0, 1.0 / 100.0, 0.0),
            ev100_from_camera(0.0, 1.0 / 100.0, 100.0),
        ] {
            assert!(ev.is_finite(), "degenerate settings produced {ev}");
        }
    }
}
