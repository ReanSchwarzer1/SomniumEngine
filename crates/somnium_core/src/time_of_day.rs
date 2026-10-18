//! Phase CONTROL-L: the day cycle.
//!
//! # The shape every surveyed engine converged on
//!
//! Phase CONTROL §6.3 records it: **one scalar driver, named presets, and
//! curves mapping the driver onto parameters.** This module is that, and
//! nothing more. The driver is the hour of day; the curves are ordinary
//! CONTROL-K [`Curve`]s and [`Gradient`]s declared in the component's schema,
//! so they are authored in the same Details panel as everything else and
//! round-trip through the same serializer.
//!
//! # Why the sun is analytic
//!
//! Unreal's SunSky model, chosen for the reason the plan gives: it is cheap,
//! immediately credible, and it hands you latitude, longitude and date for
//! free. A hand-authored azimuth track cannot tell you that the sun rises
//! south of east in a northern winter; the NOAA solar-position equations can,
//! and they are forty lines.
//!
//! The formulation here is NOAA's *Solar Calculation Details*: the fractional
//! year, the equation of time, the solar declination, then the hour angle.
//! Accuracy is well inside a degree for the years and latitudes an editor
//! cares about, which is far better than anybody authoring a sunset needs.
//!
//! # What it drives, and what it deliberately does not write
//!
//! The driver writes the **sun entity's rotation, colour and intensity**,
//! because those are genuinely derived from `(hour, day, latitude, longitude)`
//! and recomputing them on load reproduces them exactly. It pushes fog
//! density, exposure compensation and cloud coverage **straight to the
//! renderer** without touching `PostProcessComponent`, because those fields
//! are authored elsewhere and a driver that wrote them every frame would fight
//! the inspector and mark the scene dirty sixty times a second.
//!
//! Rotating the sun by hand while the cycle is enabled is therefore overridden
//! on the next frame. That is the correct behaviour and the component's
//! `enabled` flag is how you stop it.

use somnium_ecs::Component;
use somnium_ecs::curve::{Curve, CurveKey, Gradient, GradientStop, Interpolation};

/// Hours in a day, as a float, because every conversion here wants it that way.
const HOURS: f32 = 24.0;

/// The scene's day cycle.
///
/// One per scene. `enabled` off leaves every light exactly as authored, which
/// is what every scene that predates this component gets.
#[derive(Debug, Clone, PartialEq)]
pub struct TimeOfDayComponent {
    /// Drive the sun from this component. Off leaves lights alone entirely.
    pub enabled: bool,
    /// Local clock time, `0..24`. The one scalar driver.
    pub hour: f32,
    /// Day of the year, `1..=366`. Decides the sun's declination, which is
    /// what makes a winter sun low and a summer sun high at the same hour.
    pub day_of_year: i32,
    /// Degrees north. Negative is south.
    pub latitude: f32,
    /// Degrees east. Negative is west. The time zone is derived from this
    /// rather than authored, so a longitude change moves noon where it should.
    pub longitude: f32,
    /// Game hours advanced per real second while Play is running. Zero freezes
    /// the clock, which is the default: a scene should not start moving the
    /// moment somebody presses Play unless that was asked for.
    pub timescale: f32,
    /// Sun tint across the day, sampled at `hour / 24`.
    ///
    /// This is *tint*, not the sunset colour — that comes out of
    /// [`crate::sun::transmittance`]'s Rayleigh integration on its own, which
    /// is why the default ramp is nearly white throughout. Authoring a heavy
    /// orange here on top of the physics is how a sunset ends up looking like
    /// a filter.
    pub sun_color: Gradient,
    /// Sun illuminance in lux across the day.
    pub sun_intensity: Curve,
    /// Fog extinction per metre across the day. An empty curve leaves the
    /// `PostProcessComponent`'s authored value alone.
    pub fog_density: Curve,
    /// Exposure compensation in stops across the day. Empty leaves the
    /// authored value alone.
    pub exposure_compensation: Curve,
    /// Cloud coverage `0..1` across the day. Consumed by CONTROL-M's sky.
    /// Empty leaves the sky component's authored coverage alone.
    pub cloud_coverage: Curve,
    /// A sun that does not cross the sky: a world locked to its star, a
    /// stylised scene, a game that places the sun itself every frame. When on,
    /// the sun stands at `fixed_azimuth`/`fixed_elevation` with
    /// `fixed_illuminance` lux and a noon tint; the hour still drives the fog,
    /// exposure and cloud tracks. Rimeborn writes these from its world frame.
    pub fixed_sun: bool,
    /// Degrees in [`sun_rotation`]'s convention (0 toward +Z, 90 toward +X);
    /// [`azimuth_elevation_from_direction`] converts a direction.
    pub fixed_azimuth: f32,
    /// Degrees above the horizon. Below zero the sun has set and
    /// `sun::transmittance` takes the light to nothing, as for the Earth model.
    pub fixed_elevation: f32,
    /// Illuminance of the fixed sun when nothing covers it, lux.
    pub fixed_illuminance: f32,
    /// How much of the sun is covered, `0..1`: an eclipse, or a shade passing
    /// overhead. Scales the sun's illuminance, and the sky with it (the sky dome
    /// is lit by the sun's own output), in either sun model.
    pub eclipse: f32,
    /// A moonless night sky's glow at the zenith, cd/m² (`NightSky`: drawn into the sky, the ambient light and the
    /// reflections). 0 draws none. Rimeborn's Undersun uses about 0.03 under the Lid.
    pub night_glow: f32,
    /// The glow's colour (linear RGB).
    pub night_tint: glam::Vec3,
    /// Aurora low on the side away from the sun, 0..1.
    pub night_aurora: f32,
    /// The Halo Road (the Lid plates' lit edges), 0..1.
    pub night_halo_road: f32,
    /// The Thread (a line of light from the horizon below the sun toward it), 0..1.
    pub night_thread: f32,
}

impl Component for TimeOfDayComponent {}

impl Default for TimeOfDayComponent {
    fn default() -> Self {
        Self {
            enabled: true,
            hour: 12.0,
            day_of_year: 172, // the June solstice: the longest day to look at
            latitude: 45.0,
            longitude: 0.0,
            timescale: 0.0,
            sun_color: default_sun_color(),
            sun_intensity: default_sun_intensity(),
            fog_density: Curve::empty(),
            exposure_compensation: Curve::empty(),
            cloud_coverage: Curve::empty(),
            fixed_sun: false,
            fixed_azimuth: 180.0,
            fixed_elevation: 45.0,
            fixed_illuminance: crate::light_units::lux::DIRECT_SUNLIGHT,
            eclipse: 0.0,
            night_glow: 0.0,
            night_tint: glam::Vec3::new(0.55, 0.65, 1.0),
            night_aurora: 0.0,
            night_halo_road: 0.0,
            night_thread: 0.0,
        }
    }
}

/// The shipped sun tint ramp: cool before dawn, neutral through the day, warm
/// at the ends. Deliberately gentle — see [`TimeOfDayComponent::sun_color`].
fn default_sun_color() -> Gradient {
    Gradient::from_stops(vec![
        GradientStop::new(0.0, [0.55, 0.62, 0.85, 1.0]), // midnight
        GradientStop::new(5.0 / HOURS, [0.85, 0.72, 0.62, 1.0]), // first light
        GradientStop::new(8.0 / HOURS, [1.0, 0.97, 0.92, 1.0]), // morning
        GradientStop::new(12.0 / HOURS, [1.0, 1.0, 1.0, 1.0]), // noon
        GradientStop::new(17.0 / HOURS, [1.0, 0.95, 0.88, 1.0]), // afternoon
        GradientStop::new(19.5 / HOURS, [1.0, 0.78, 0.55, 1.0]), // golden hour
        GradientStop::new(21.0 / HOURS, [0.72, 0.62, 0.72, 1.0]), // dusk
        GradientStop::new(1.0, [0.55, 0.62, 0.85, 1.0]), // wraps to midnight
    ])
}

/// The shipped intensity track, in lux.
///
/// Note that this is *not* the sunset fade: `sun::transmittance` already takes
/// the light to zero below the horizon, from physics. This track is the
/// authored part — how bright the sun is when it *is* up — and it is nearly
/// flat on purpose.
fn default_sun_intensity() -> Curve {
    Curve::from_keys(vec![
        CurveKey {
            interpolation: Interpolation::Smooth,
            ..CurveKey::new(0.0, 0.0)
        },
        CurveKey {
            interpolation: Interpolation::Smooth,
            ..CurveKey::new(6.0, 20_000.0)
        },
        CurveKey {
            interpolation: Interpolation::Smooth,
            ..CurveKey::new(12.0, crate::light_units::lux::DIRECT_SUNLIGHT)
        },
        CurveKey {
            interpolation: Interpolation::Smooth,
            ..CurveKey::new(18.0, 20_000.0)
        },
        CurveKey::new(24.0, 0.0),
    ])
}

/// The named times CONTROL-L ships, as commands rather than as an enum field.
///
/// A preset that was a component field would be a second description of the
/// hour — set the preset to Noon, drag the hour to 3 am, and the field now
/// lies. A command sets the hour and stops existing, which is the whole
/// difference.
///
/// The table itself lives in the command registry (Seam 6) so the label a user
/// reads and the hour the sun moves to are one declaration.
pub use somnium_ui::commands::TIME_PRESETS as PRESETS;

/// The hour a named preset means, or `None` for an unknown id.
#[must_use]
pub fn preset_hour(id: &str) -> Option<f32> {
    PRESETS
        .iter()
        .find(|(preset, _, _)| *preset == id)
        .map(|(_, _, hour)| *hour)
}

/// Where the sun is, in degrees.
///
/// Returns `(azimuth, elevation)`: azimuth clockwise from true north,
/// elevation above the horizon. NOAA's *Solar Calculation Details*.
#[must_use]
pub fn solar_position(day_of_year: i32, hour: f32, latitude: f32, longitude: f32) -> (f32, f32) {
    let day = day_of_year.clamp(1, 366);
    let hour = hour.rem_euclid(HOURS);

    // Fractional year, radians.
    #[allow(clippy::cast_precision_loss)]
    let gamma = std::f32::consts::TAU / 365.0 * ((day - 1) as f32 + (hour - 12.0) / HOURS);

    // Equation of time, minutes. The difference between clock noon and solar
    // noon, and the reason an analemma is a figure of eight.
    let eqtime = 229.18
        * (0.000_075 + 0.001_868 * gamma.cos()
            - 0.032_077 * gamma.sin()
            - 0.014_615 * (2.0 * gamma).cos()
            - 0.040_849 * (2.0 * gamma).sin());

    // Solar declination, radians.
    let decl = 0.006_918 - 0.399_912 * gamma.cos() + 0.070_257 * gamma.sin()
        - 0.006_758 * (2.0 * gamma).cos()
        + 0.000_907 * (2.0 * gamma).sin()
        - 0.002_697 * (3.0 * gamma).cos()
        + 0.001_48 * (3.0 * gamma).sin();

    // Time zone derived from longitude rather than authored: an author moving
    // the marker across a map should not also have to remember to change a
    // zone offset, and the two disagreeing is a bug that looks like the sun
    // being an hour off for no reason.
    let timezone = (longitude / 15.0).round();
    let true_solar_minutes = hour * 60.0 + eqtime + 4.0 * longitude - 60.0 * timezone;
    let hour_angle_deg = true_solar_minutes / 4.0 - 180.0;
    let ha = hour_angle_deg.to_radians();

    let lat = latitude.clamp(-89.9, 89.9).to_radians();
    let cos_zenith = (lat.sin() * decl.sin() + lat.cos() * decl.cos() * ha.cos()).clamp(-1.0, 1.0);
    let zenith = cos_zenith.acos();
    let elevation = 90.0 - zenith.to_degrees();

    // Azimuth from the zenith angle. The denominator vanishes at the poles and
    // exactly at the zenith, so it is guarded rather than trusted.
    let denom = lat.cos() * zenith.sin();
    let azimuth = if denom.abs() < 1e-6 {
        180.0
    } else {
        let cos_az = ((lat.sin() * cos_zenith - decl.sin()) / denom).clamp(-1.0, 1.0);
        let az = 180.0 - cos_az.acos().to_degrees();
        if hour_angle_deg > 0.0 { 360.0 - az } else { az }
    };

    (azimuth.rem_euclid(360.0), elevation)
}

/// The rotation a directional light needs to shine from `(azimuth, elevation)`.
///
/// Matches the convention `map.rs` established for `SOMNIUM_SUN_AZIMUTH` /
/// `SOMNIUM_SUN_ELEVATION` exactly, so the env vars and this driver cannot
/// disagree about which way is which — which is the whole point of demoting
/// them from "the only way to place the sun" to "an override of a real
/// system".
#[must_use]
pub fn sun_rotation(azimuth_deg: f32, elevation_deg: f32) -> glam::Quat {
    glam::Quat::from_euler(
        glam::EulerRot::YXZ,
        azimuth_deg.to_radians(),
        -elevation_deg.to_radians(),
        0.0,
    )
}

/// The `(azimuth, elevation)` in degrees of a direction *toward* the sun, in
/// the convention [`sun_rotation`] uses: azimuth 0 puts the sun toward +Z and
/// 90 toward +X (the toward-sun vector is `(sin az cos el, sin el, cos az cos
/// el)`), elevation above the horizon. A game with its own idea of north
/// passes its direction through this rather than reasoning about azimuths.
#[must_use]
pub fn azimuth_elevation_from_direction(toward_sun: glam::Vec3) -> (f32, f32) {
    let d = toward_sun.normalize_or(glam::Vec3::Y);
    let azimuth = d.x.atan2(d.z).to_degrees().rem_euclid(360.0);
    (azimuth, d.y.clamp(-1.0, 1.0).asin().to_degrees())
}

/// Everything the day cycle produces for one instant.
///
/// Returned as a value rather than applied in place so it can be tested
/// without a world, a renderer or a window.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DayState {
    /// Azimuth clockwise from north, degrees.
    pub azimuth_deg: f32,
    /// Elevation above the horizon, degrees.
    pub elevation_deg: f32,
    /// Light rotation for a `-Z`-forward directional light.
    pub rotation: glam::Quat,
    /// Sun tint, linear RGB.
    pub color: glam::Vec3,
    /// Sun illuminance, lux.
    pub intensity: f32,
    /// Fog extinction per metre, or `None` when the track is unauthored.
    pub fog_density: Option<f32>,
    /// Exposure compensation in stops, or `None` when unauthored.
    pub exposure_compensation: Option<f32>,
    /// Cloud coverage `0..1`, or `None` when unauthored.
    pub cloud_coverage: Option<f32>,
    /// The moonless night sky (`TimeOfDayComponent::night_*`); all zero is none.
    pub night: somnium_renderer::pass::ibl::NightSky,
}

impl TimeOfDayComponent {
    /// Advance the clock by `dt` real seconds and wrap it into `0..24`.
    pub fn advance(&mut self, dt: f32) {
        if self.timescale != 0.0 {
            self.hour = (self.hour + dt * self.timescale).rem_euclid(HOURS);
        }
    }

    /// Evaluate every track at the current hour.
    #[must_use]
    pub fn evaluate(&self) -> DayState {
        let hour = self.hour.rem_euclid(HOURS);
        let (azimuth_deg, elevation_deg) = if self.fixed_sun {
            (self.fixed_azimuth.rem_euclid(360.0), self.fixed_elevation.clamp(-90.0, 90.0))
        } else {
            solar_position(self.day_of_year, hour, self.latitude, self.longitude)
        };
        // A fixed sun keeps the noon tint: its colour at a low elevation comes
        // from the transmittance physics, not from a clock it does not have.
        let tint = self
            .sun_color
            .evaluate(if self.fixed_sun { 0.5 } else { hour / HOURS });
        let uncovered = 1.0 - self.eclipse.clamp(0.0, 1.0);
        let night = somnium_renderer::pass::ibl::NightSky {
            glow: self.night_glow.max(0.0),
            aurora: self.night_aurora.clamp(0.0, 1.0),
            halo_road: self.night_halo_road.clamp(0.0, 1.0),
            thread: self.night_thread.clamp(0.0, 1.0),
            tint: self.night_tint.to_array(),
        };
        DayState {
            night,
            azimuth_deg,
            elevation_deg,
            rotation: sun_rotation(azimuth_deg, elevation_deg),
            color: glam::Vec3::new(tint[0], tint[1], tint[2]),
            // An unauthored intensity track means "leave the authored
            // illuminance alone", which a zero would not: zero is night. An
            // eclipse is the exception: covering the sun is a statement about
            // its output, so it needs a value to scale.
            intensity: if self.fixed_sun {
                self.fixed_illuminance.max(0.0) * uncovered
            } else if self.sun_intensity.is_empty() {
                if uncovered < 1.0 {
                    crate::light_units::lux::DIRECT_SUNLIGHT * uncovered
                } else {
                    f32::NAN
                }
            } else {
                self.sun_intensity.evaluate(hour).max(0.0) * uncovered
            },
            fog_density: (!self.fog_density.is_empty())
                .then(|| self.fog_density.evaluate(hour).max(0.0)),
            exposure_compensation: (!self.exposure_compensation.is_empty())
                .then(|| self.exposure_compensation.evaluate(hour)),
            cloud_coverage: (!self.cloud_coverage.is_empty())
                .then(|| self.cloud_coverage.evaluate(hour).clamp(0.0, 1.0)),
        }
    }

    /// Format the clock as `HH:MM`, for the context bar and the status line.
    #[must_use]
    pub fn clock_text(&self) -> String {
        let hour = self.hour.rem_euclid(HOURS);
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let h = hour.floor() as u32;
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let m = ((hour - hour.floor()) * 60.0).round() as u32;
        // 59.7 minutes rounds to 60, which is not a minute of any hour.
        if m >= 60 {
            format!("{:02}:00", (h + 1) % 24)
        } else {
            format!("{h:02}:{m:02}")
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Noon at the equator on an equinox puts the sun essentially overhead.
    /// The check that the declination and hour-angle terms are not swapped.
    #[test]
    fn the_equinox_noon_sun_is_overhead_at_the_equator() {
        let (_, elevation) = solar_position(80, 12.0, 0.0, 0.0);
        assert!(elevation > 88.0, "elevation was {elevation}");
    }

    /// Midnight is the same instant upside down. A sign error in the hour
    /// angle shows up here and nowhere else.
    #[test]
    fn midnight_puts_the_sun_below_the_horizon() {
        for latitude in [-40.0, 0.0, 40.0] {
            let (_, elevation) = solar_position(80, 0.0, latitude, 0.0);
            assert!(elevation < 0.0, "lat {latitude} elevation {elevation}");
        }
    }

    /// The June solstice is high in the north and low in the south on the same
    /// day — which is the whole reason `day_of_year` is a field.
    #[test]
    fn the_solstice_favours_one_hemisphere() {
        let (_, north) = solar_position(172, 12.0, 50.0, 0.0);
        let (_, south) = solar_position(172, 12.0, -50.0, 0.0);
        assert!(north > south + 40.0, "north {north} south {south}");
    }

    /// Morning sun in the east, afternoon sun in the west, northern latitude.
    #[test]
    fn the_sun_crosses_from_east_to_west() {
        let (morning, _) = solar_position(172, 8.0, 45.0, 0.0);
        let (afternoon, _) = solar_position(172, 16.0, 45.0, 0.0);
        assert!(
            (45.0..135.0).contains(&morning),
            "morning azimuth {morning} is not eastward"
        );
        assert!(
            (225.0..315.0).contains(&afternoon),
            "afternoon azimuth {afternoon} is not westward"
        );
    }

    /// Elevation must be a smooth arc, not a sawtooth: a discontinuity here is
    /// a visible flash in the sky when the hour crosses it.
    #[test]
    fn elevation_is_continuous_across_the_whole_day() {
        let mut previous = solar_position(172, 0.0, 45.0, 0.0).1;
        let mut hour = 0.05;
        while hour <= 24.0 {
            let elevation = solar_position(172, hour, 45.0, 0.0).1;
            assert!(
                (elevation - previous).abs() < 1.5,
                "jump at hour {hour}: {previous} → {elevation}"
            );
            previous = elevation;
            hour += 0.05;
        }
    }

    /// The rotation this module produces must agree with `sun::transmittance`'s
    /// idea of "up", or a sun the maths says is above the horizon would be lit
    /// as though it had set. This is the exact bug 25M existed to fix, one
    /// layer up.
    #[test]
    fn a_high_sun_points_its_light_downward() {
        let rotation = sun_rotation(180.0, 60.0);
        let to_light = -rotation.mul_vec3(glam::Vec3::NEG_Z);
        assert!(to_light.y > 0.8, "to_light was {to_light}");
        assert!(crate::sun::transmittance(to_light.y, 0.0).length() > 0.5);
    }

    #[test]
    fn a_set_sun_delivers_no_direct_light() {
        let rotation = sun_rotation(0.0, -20.0);
        let to_light = -rotation.mul_vec3(glam::Vec3::NEG_Z);
        assert!(to_light.y < 0.0);
        assert_eq!(crate::sun::transmittance(to_light.y, 0.0), glam::Vec3::ZERO);
    }

    #[test]
    fn the_clock_wraps_instead_of_running_off_the_end() {
        let mut tod = TimeOfDayComponent {
            hour: 23.5,
            timescale: 1.0,
            ..TimeOfDayComponent::default()
        };
        tod.advance(1.0); // one game hour at 1 h/s
        assert!((tod.hour - 0.5).abs() < 1e-3, "hour was {}", tod.hour);
    }

    #[test]
    fn a_zero_timescale_freezes_the_clock() {
        let mut tod = TimeOfDayComponent::default();
        let before = tod.hour;
        tod.advance(10.0);
        assert_eq!(tod.hour, before);
    }

    #[test]
    fn the_clock_reads_as_a_time_not_a_float() {
        let mut tod = TimeOfDayComponent::default();
        tod.hour = 19.25;
        assert_eq!(tod.clock_text(), "19:15");
        tod.hour = 23.999;
        assert_eq!(tod.clock_text(), "00:00");
    }

    /// An unauthored track must leave its target alone, and the difference
    /// between "unauthored" and "authored zero" has to survive: a zero fog
    /// density is clear air, not "keep whatever was there".
    #[test]
    fn unauthored_tracks_report_nothing_rather_than_zero() {
        let state = TimeOfDayComponent::default().evaluate();
        assert_eq!(state.fog_density, None);
        assert_eq!(state.exposure_compensation, None);
        assert_eq!(state.cloud_coverage, None);

        let driven = TimeOfDayComponent {
            fog_density: Curve::constant(0.0),
            ..TimeOfDayComponent::default()
        }
        .evaluate();
        assert_eq!(driven.fog_density, Some(0.0));
    }

    #[test]
    fn every_preset_is_a_real_hour_and_is_findable_by_id() {
        for (id, label, hour) in PRESETS {
            assert!(!id.is_empty() && !label.is_empty());
            assert!((0.0..24.0).contains(&hour), "{label} at {hour}");
            assert_eq!(preset_hour(id), Some(hour));
        }
        assert_eq!(preset_hour("teatime"), None);
    }

    /// The shipped tracks must not themselves be the thing that breaks the
    /// day: a default component evaluates to a lit, finite noon.
    #[test]
    fn the_default_cycle_is_a_bright_noon() {
        let state = TimeOfDayComponent::default().evaluate();
        assert!(state.elevation_deg > 40.0, "{}", state.elevation_deg);
        assert!(state.intensity > 50_000.0, "{}", state.intensity);
        assert!(state.color.min_element() > 0.5);
    }

    /// The direction helper and the light rotation agree about which way is
    /// which: a light built from a direction shines back along it.
    #[test]
    fn a_direction_round_trips_through_the_light_rotation() {
        for toward in [
            glam::Vec3::new(1.0, 0.5, 0.0),
            glam::Vec3::new(-0.3, 0.6, -0.7),
            glam::Vec3::new(0.2, 0.1, 0.9),
        ] {
            let toward = toward.normalize();
            let (az, el) = azimuth_elevation_from_direction(toward);
            let forward = sun_rotation(az, el) * glam::Vec3::NEG_Z;
            assert!(forward.dot(-toward) > 0.9999, "{toward} -> ({az}, {el}) -> {forward}");
        }
        // +X is 90 degrees and +Z is 0, as `sun_rotation` builds them.
        assert!((azimuth_elevation_from_direction(glam::Vec3::X).0 - 90.0).abs() < 1e-4);
        assert!(azimuth_elevation_from_direction(glam::Vec3::Z).0.abs() < 1e-4);
    }

    /// A fixed sun ignores the clock for its place and brightness; the hour
    /// still drives the other tracks.
    #[test]
    fn a_fixed_sun_stands_still_through_the_day() {
        let mut tod = TimeOfDayComponent {
            fixed_sun: true,
            fixed_azimuth: 300.0,
            fixed_elevation: 35.0,
            fixed_illuminance: 60_000.0,
            fog_density: Curve::constant(0.001),
            ..TimeOfDayComponent::default()
        };
        for hour in [0.0, 6.0, 12.0, 21.0] {
            tod.hour = hour;
            let state = tod.evaluate();
            assert!((state.azimuth_deg - 300.0).abs() < 1e-4);
            assert!((state.elevation_deg - 35.0).abs() < 1e-4);
            assert!((state.intensity - 60_000.0).abs() < 1e-2);
            assert_eq!(state.fog_density, Some(0.001));
        }
    }

    /// Covering the sun scales its light in either model, and an Earth sun
    /// with no authored intensity still gets a value an eclipse can scale.
    #[test]
    fn an_eclipse_covers_the_sun_in_either_model() {
        let fixed = TimeOfDayComponent {
            fixed_sun: true,
            fixed_illuminance: 80_000.0,
            eclipse: 0.75,
            ..TimeOfDayComponent::default()
        }
        .evaluate();
        assert!((fixed.intensity - 20_000.0).abs() < 1e-2);

        let earth = TimeOfDayComponent { eclipse: 1.0, ..TimeOfDayComponent::default() }.evaluate();
        assert!(earth.intensity.abs() < 1e-3);

        let unauthored = TimeOfDayComponent {
            sun_intensity: Curve::empty(),
            eclipse: 0.5,
            ..TimeOfDayComponent::default()
        }
        .evaluate();
        assert!(unauthored.intensity.is_finite());
        let clear = TimeOfDayComponent { sun_intensity: Curve::empty(), ..TimeOfDayComponent::default() }.evaluate();
        assert!(clear.intensity.is_nan(), "no eclipse and no track still means: leave the light alone");
    }
}
