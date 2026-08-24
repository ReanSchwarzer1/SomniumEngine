//! Phase CONTROL-N: weather, and the wetness it leaves behind.
//!
//! # Why this is one sub-phase and not three
//!
//! Because the chain is the point. Coverage drives precipitation, precipitation
//! drives wetness, wetness drives the surface — and a version of this built as
//! three independent features would have three sliders where it needs one
//! cause. The phase document says so and this module is shaped by it: the
//! component below is a small set of *causes*, and everything downstream is
//! derived.
//!
//! # Wetness follows Lagarde, not feel
//!
//! Sébastien Lagarde's *Water drop 3a/3b* is the reference (§6.3), and four of
//! its findings are load-bearing here:
//!
//! 1. **Two time constants, not one.** A surface wets quickly and dries slowly,
//!    and those are different numbers. [`WeatherComponent::wetting_seconds`] and
//!    [`WeatherComponent::drying_seconds`] are both authored.
//! 2. **Specular recovers before diffuse.** A puddle stops *looking* wet before
//!    it stops being dark, so the two are tracked separately and the specular
//!    term dries faster by an authored ratio. One scalar could not express it.
//! 3. **Porosity is a material channel**, not a wetness-only input: it is the
//!    one value that says how much water a surface can take up, and it drives
//!    ageing and pollution too. It lives on the material, beside roughness.
//! 4. **No separate wet texture set.** Diffuse darkening, a specular boost, and
//!    an accumulated-water term that progressively flattens the normal — with
//!    puddles as the flat-normal limit case. The albedo darkening is
//!    **non-linear in the base albedo**, not a multiply, because a multiply
//!    darkens a white surface as much as a black one and water does not.
//!
//! # What is a target and what is state
//!
//! The component holds *targets*. The live values — the two wetness scalars,
//! the transitioning precipitation rate — are engine state, recomputed every
//! frame from the targets and never saved. That split is what makes "named
//! states with an explicit transition duration" work: a preset writes targets,
//! and the transition is the driver walking toward them.

use somnium_ecs::Component;

/// The scene's weather.
///
/// Lives on the Environment entity beside [`SkyComponent`] and
/// [`TimeOfDayComponent`], because it is the middle of their chain.
///
/// [`SkyComponent`]: crate::sky::SkyComponent
/// [`TimeOfDayComponent`]: crate::time_of_day::TimeOfDayComponent
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct WeatherComponent {
    /// Drive wetness, wind and precipitation from this component.
    ///
    /// Off leaves the terrain's authored wetness, the water's authored wind and
    /// every material exactly as they are — which is what every scene that
    /// predates this component gets.
    pub enabled: bool,
    /// Precipitation rate, `0..1`. Zero is dry.
    pub precipitation: f32,
    /// Air temperature in degrees Celsius.
    ///
    /// The rain/snow switch, and nothing else. Below freezing the same rate
    /// falls as snow: slower, larger, less wind-sheared, and it does not wet
    /// the ground.
    pub temperature_c: f32,
    /// Wind speed in metres per second.
    ///
    /// **The scene's one wind.** Cloud advection, the ocean spectrum and
    /// precipitation shear all read this rather than each carrying a private
    /// constant, which is what stops a storm blowing three directions at once.
    pub wind_speed: f32,
    /// Wind bearing in degrees, clockwise from +Z. Same convention as the
    /// sun's azimuth, so the two read the same way on a compass.
    pub wind_direction_deg: f32,
    /// How wet the world gets at full precipitation, `0..1`.
    ///
    /// Separate from the rate because a light drizzle on a hot road never gets
    /// past damp, and a rate of 0.2 that eventually soaked everything would be
    /// wrong in a way no amount of waiting fixes.
    pub wetness_target: f32,
    /// Seconds for the diffuse term to reach 63% of its target while wetting.
    pub wetting_seconds: f32,
    /// Seconds for the diffuse term to fall to 37% of its value while drying.
    ///
    /// Longer than [`Self::wetting_seconds`] by default, because rain arrives
    /// faster than it evaporates and a symmetric model reads as a mistake.
    pub drying_seconds: f32,
    /// How much faster the specular term dries than the diffuse one.
    ///
    /// Lagarde's finding: a surface stops looking wet before it stops looking
    /// dark. `1.0` collapses the two and is the wrong answer kept reachable.
    pub specular_dry_ratio: f32,
    /// Standing water at full wetness, `0..1`.
    ///
    /// The accumulated-water term. It progressively flattens the surface
    /// normal, with a mirror-flat puddle as the limit case, rather than
    /// switching in a separate puddle material.
    pub puddles: f32,
    /// Strength of rain ripples on water surfaces, `0..1`.
    pub ripple_strength: f32,
    /// Seconds a preset transition takes.
    ///
    /// Ultra Dynamic Sky's model: weather is named states plus an explicit
    /// duration, not a pile of sliders somebody eases by hand.
    pub transition_seconds: f32,
    /// Particles per second at full precipitation.
    pub particle_rate: f32,
}

impl Component for WeatherComponent {}

impl Default for WeatherComponent {
    fn default() -> Self {
        Self {
            enabled: false,
            precipitation: 0.0,
            temperature_c: 14.0,
            wind_speed: 8.0,
            wind_direction_deg: 45.0,
            wetness_target: 1.0,
            wetting_seconds: 12.0,
            drying_seconds: 90.0,
            specular_dry_ratio: 3.0,
            puddles: 0.35,
            ripple_strength: 0.6,
            transition_seconds: 8.0,
            particle_rate: 6_000.0,
        }
    }
}

/// Precipitation that is actually falling.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Precipitation {
    /// Nothing is falling.
    None,
    /// Liquid water. Wets the ground.
    Rain,
    /// Frozen. Falls slower, shears less, and does **not** wet the ground —
    /// which is why the wetness target is gated on it rather than on the rate.
    Snow,
}

/// The live weather, recomputed every frame and never saved.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct WeatherState {
    /// What is falling right now.
    pub precipitation: Precipitation,
    /// Rate after the transition easing, `0..1`.
    pub rate: f32,
    /// Wind as a world vector in metres per second, XZ.
    pub wind: [f32; 2],
    /// Diffuse wetness — how *dark* the surface is, `0..1`.
    pub wet_diffuse: f32,
    /// Specular wetness — how *glossy* the surface is, `0..1`. Recovers first.
    pub wet_specular: f32,
    /// Standing water, `0..1`. Flattens the normal.
    pub puddles: f32,
    /// Rain-ripple strength for water surfaces, `0..1`.
    pub ripples: f32,
}

impl Default for WeatherState {
    fn default() -> Self {
        Self {
            precipitation: Precipitation::None,
            rate: 0.0,
            wind: [0.0, 0.0],
            wet_diffuse: 0.0,
            wet_specular: 0.0,
            puddles: 0.0,
            ripples: 0.0,
        }
    }
}

/// Exponential approach to `target` with time constant `tau`, over `dt`.
///
/// Framerate-independent by construction — `1 - exp(-dt/tau)` rather than a
/// fixed lerp factor — because a wetness that dried faster on a fast machine
/// would be a bug nobody could reproduce.
#[must_use]
pub fn approach(current: f32, target: f32, tau: f32, dt: f32) -> f32 {
    if tau <= 0.0 || dt <= 0.0 {
        return target;
    }
    current + (target - current) * (1.0 - (-dt / tau).exp())
}

impl WeatherComponent {
    /// Wind as a world vector.
    #[must_use]
    pub fn wind_vector(&self) -> [f32; 2] {
        let bearing = self.wind_direction_deg.to_radians();
        [
            self.wind_speed * bearing.sin(),
            self.wind_speed * bearing.cos(),
        ]
    }

    /// What is falling, given the rate and the temperature.
    #[must_use]
    pub fn falling(&self, rate: f32) -> Precipitation {
        if rate <= 0.001 {
            Precipitation::None
        } else if self.temperature_c <= 0.0 {
            Precipitation::Snow
        } else {
            Precipitation::Rain
        }
    }

    /// Advance the live state by `dt` seconds.
    ///
    /// The whole of the chain in one function, so it can be tested against a
    /// clock without a world, a renderer or a window.
    #[must_use]
    pub fn step(&self, previous: WeatherState, dt: f32) -> WeatherState {
        if !self.enabled {
            // Disabled does not mean frozen: the world dries out and the rain
            // eases off, so switching weather off looks like weather ending
            // rather than like a pause.
            let dry = self.drying_seconds.max(0.001);
            return WeatherState {
                precipitation: Precipitation::None,
                rate: approach(previous.rate, 0.0, self.transition_seconds.max(0.001), dt),
                wind: [0.0, 0.0],
                wet_diffuse: approach(previous.wet_diffuse, 0.0, dry, dt),
                wet_specular: approach(
                    previous.wet_specular,
                    0.0,
                    dry / self.specular_dry_ratio.max(0.01),
                    dt,
                ),
                puddles: approach(previous.puddles, 0.0, dry, dt),
                ripples: approach(
                    previous.ripples,
                    0.0,
                    self.transition_seconds.max(0.001),
                    dt,
                ),
            };
        }

        let rate = approach(
            previous.rate,
            self.precipitation.clamp(0.0, 1.0),
            self.transition_seconds.max(0.001),
            dt,
        );
        let falling = self.falling(rate);

        // Snow does not wet the ground. Gating on the *kind* rather than on
        // the rate is the difference between a blizzard leaving a dry road and
        // a blizzard leaving a soaked one.
        let target = if falling == Precipitation::Rain {
            (self.wetness_target.clamp(0.0, 1.0) * rate).clamp(0.0, 1.0)
        } else {
            0.0
        };

        // Two constants: wetting is fast, drying is slow, and which one applies
        // depends on which way the value is moving.
        let diffuse_tau = if target > previous.wet_diffuse {
            self.wetting_seconds.max(0.001)
        } else {
            self.drying_seconds.max(0.001)
        };
        let wet_diffuse = approach(previous.wet_diffuse, target, diffuse_tau, dt);

        // Specular wets at the same speed and dries faster, which is the
        // asymmetry Lagarde measured: a road stops shining long before it
        // stops being dark.
        let specular_tau = if target > previous.wet_specular {
            self.wetting_seconds.max(0.001)
        } else {
            self.drying_seconds.max(0.001) / self.specular_dry_ratio.max(0.01)
        };
        let wet_specular = approach(previous.wet_specular, target, specular_tau, dt);

        // Standing water lags the surface film: a puddle takes far longer to
        // form and far longer to go than a wet flagstone.
        let puddle_target = self.puddles.clamp(0.0, 1.0) * wet_diffuse;
        let puddle_tau = if puddle_target > previous.puddles {
            self.wetting_seconds.max(0.001) * 3.0
        } else {
            self.drying_seconds.max(0.001) * 2.0
        };

        WeatherState {
            precipitation: falling,
            rate,
            wind: self.wind_vector(),
            wet_diffuse,
            wet_specular,
            puddles: approach(previous.puddles, puddle_target, puddle_tau, dt),
            ripples: if falling == Precipitation::Rain {
                self.ripple_strength.clamp(0.0, 1.0) * rate
            } else {
                0.0
            },
        }
    }
}

/// The subset of fields a weather preset sets.
#[derive(Debug, Clone, Copy)]
pub struct WeatherPreset {
    /// Precipitation rate, `0..1`.
    pub precipitation: f32,
    /// Air temperature in Celsius — the rain/snow switch.
    pub temperature_c: f32,
    /// Wind speed in metres per second.
    pub wind_speed: f32,
    /// Standing water at full wetness.
    pub puddles: f32,
    /// Rain-ripple strength on water.
    pub ripple_strength: f32,
}

/// The named states CONTROL-N ships.
///
/// Deliberately paired with [`crate::sky::PRESETS`] by name where they overlap:
/// "Storm" here and "Storm" there are the two halves of one weather, and the
/// `editor.weather.storm` command applies both.
pub const PRESETS: [(&str, &str, WeatherPreset); 4] = [
    (
        "clear",
        "Clear",
        WeatherPreset {
            precipitation: 0.0,
            temperature_c: 18.0,
            wind_speed: 5.0,
            puddles: 0.0,
            ripple_strength: 0.0,
        },
    ),
    (
        "drizzle",
        "Drizzle",
        WeatherPreset {
            precipitation: 0.25,
            temperature_c: 12.0,
            wind_speed: 9.0,
            puddles: 0.15,
            ripple_strength: 0.4,
        },
    ),
    (
        "storm",
        "Storm",
        WeatherPreset {
            precipitation: 1.0,
            temperature_c: 9.0,
            wind_speed: 26.0,
            puddles: 0.65,
            ripple_strength: 1.0,
        },
    ),
    (
        "snow",
        "Snow",
        WeatherPreset {
            precipitation: 0.7,
            temperature_c: -4.0,
            wind_speed: 7.0,
            puddles: 0.0,
            ripple_strength: 0.0,
        },
    ),
];

/// The sky a weather state implies.
///
/// CONTROL-N's exit criterion is that **one** preset takes Coastal from clear
/// to storm — clouds close, light drops, rain falls, the sea roughens. A
/// weather preset that left the sky clear while rain fell out of it would meet
/// the letter of the sub-phase and fail its whole point, so this mapping is
/// part of the feature rather than a convenience on top of it.
#[must_use]
pub fn sky_preset_for(id: &str) -> Option<&'static str> {
    match id {
        "clear" => Some("clear"),
        "drizzle" => Some("overcast"),
        "storm" => Some("storm"),
        "snow" => Some("overcast"),
        _ => None,
    }
}

impl WeatherComponent {
    /// Apply a named preset by id, returning false for an unknown one.
    ///
    /// Like the sky's, this does **not** touch `enabled`: choosing weather and
    /// paying for it are two decisions.
    pub fn apply_preset(&mut self, id: &str) -> bool {
        let Some((_, _, preset)) = PRESETS.iter().find(|(name, _, _)| *name == id) else {
            return false;
        };
        self.precipitation = preset.precipitation;
        self.temperature_c = preset.temperature_c;
        self.wind_speed = preset.wind_speed;
        self.puddles = preset.puddles;
        self.ripple_strength = preset.ripple_strength;
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(weather: &WeatherComponent, seconds: f32) -> WeatherState {
        let mut state = WeatherState::default();
        let dt = 1.0 / 60.0;
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let steps = (seconds / dt) as u32;
        for _ in 0..steps {
            state = weather.step(state, dt);
        }
        state
    }

    fn raining() -> WeatherComponent {
        WeatherComponent {
            enabled: true,
            precipitation: 1.0,
            transition_seconds: 1.0,
            ..WeatherComponent::default()
        }
    }

    #[test]
    fn an_approach_is_framerate_independent() {
        // The bug this prevents: a fixed lerp factor dries the world twice as
        // fast at 120 Hz, and nobody can reproduce it on the machine it was
        // reported from.
        let coarse = {
            let mut v = 0.0;
            for _ in 0..10 {
                v = approach(v, 1.0, 2.0, 0.1);
            }
            v
        };
        let fine = {
            let mut v = 0.0;
            for _ in 0..100 {
                v = approach(v, 1.0, 2.0, 0.01);
            }
            v
        };
        assert!((coarse - fine).abs() < 1e-3, "{coarse} vs {fine}");
    }

    #[test]
    fn rain_wets_the_world_and_the_world_dries_afterwards() {
        let weather = raining();
        let wet = run(&weather, 60.0);
        assert!(wet.wet_diffuse > 0.9, "{}", wet.wet_diffuse);
        assert_eq!(wet.precipitation, Precipitation::Rain);

        let clearing = WeatherComponent {
            precipitation: 0.0,
            ..weather
        };
        let mut state = wet;
        for _ in 0..(60 * 60) {
            state = clearing.step(state, 1.0 / 60.0);
        }
        assert!(
            state.wet_diffuse < 0.6,
            "a minute of drying: {}",
            state.wet_diffuse
        );
        assert_eq!(state.precipitation, Precipitation::None);
    }

    /// Lagarde's asymmetry, and the reason two scalars exist: a road stops
    /// shining before it stops being dark.
    #[test]
    fn specular_recovers_before_diffuse() {
        let weather = raining();
        let wet = run(&weather, 60.0);
        let clearing = WeatherComponent {
            precipitation: 0.0,
            ..weather
        };
        let mut state = wet;
        for _ in 0..(30 * 60) {
            state = clearing.step(state, 1.0 / 60.0);
        }
        assert!(
            state.wet_specular < state.wet_diffuse,
            "specular {} should have dried past diffuse {}",
            state.wet_specular,
            state.wet_diffuse
        );
    }

    /// Wetting is fast and drying is slow. A symmetric model reads as a bug.
    #[test]
    fn wetting_is_faster_than_drying() {
        let weather = raining();
        let after_wetting = run(&weather, 12.0).wet_diffuse;

        let soaked = run(&weather, 120.0);
        let clearing = WeatherComponent {
            precipitation: 0.0,
            ..weather
        };
        let mut state = soaked;
        for _ in 0..(12 * 60) {
            state = clearing.step(state, 1.0 / 60.0);
        }
        let dried = 1.0 - state.wet_diffuse;
        assert!(
            after_wetting > dried * 2.0,
            "wetting {after_wetting} vs drying {dried} over the same 12 s"
        );
    }

    /// Snow falls, and leaves the ground dry. Gated on the kind, not the rate.
    #[test]
    fn snow_does_not_wet_the_ground() {
        let weather = WeatherComponent {
            enabled: true,
            precipitation: 1.0,
            temperature_c: -5.0,
            transition_seconds: 1.0,
            ..WeatherComponent::default()
        };
        let state = run(&weather, 60.0);
        assert_eq!(state.precipitation, Precipitation::Snow);
        assert!(state.rate > 0.9, "it is still snowing hard");
        assert!(state.wet_diffuse < 0.01, "{}", state.wet_diffuse);
        assert_eq!(state.ripples, 0.0, "snow makes no ripples");
    }

    /// Puddles lag the film. A puddle that appeared with the first drop and
    /// vanished with the last would read as a decal being toggled.
    #[test]
    fn puddles_lag_the_surface_film() {
        let weather = raining();
        let early = run(&weather, 12.0);
        assert!(
            early.puddles < early.wet_diffuse * 0.6,
            "puddles {} vs film {}",
            early.puddles,
            early.wet_diffuse
        );
    }

    #[test]
    fn a_light_drizzle_never_soaks_the_world() {
        // Rate scales the target, so 0.2 of rain forever is still only damp.
        let weather = WeatherComponent {
            enabled: true,
            precipitation: 0.2,
            transition_seconds: 1.0,
            ..WeatherComponent::default()
        };
        let state = run(&weather, 300.0);
        assert!(state.wet_diffuse < 0.25, "{}", state.wet_diffuse);
    }

    #[test]
    fn wind_is_one_vector_read_off_a_compass() {
        let north = WeatherComponent {
            wind_direction_deg: 0.0,
            wind_speed: 10.0,
            ..WeatherComponent::default()
        };
        let [x, z] = north.wind_vector();
        assert!(x.abs() < 1e-4 && (z - 10.0).abs() < 1e-4, "{x} {z}");

        let east = WeatherComponent {
            wind_direction_deg: 90.0,
            ..north
        };
        let [x, z] = east.wind_vector();
        assert!((x - 10.0).abs() < 1e-4 && z.abs() < 1e-4, "{x} {z}");
    }

    /// Switching weather off must look like weather ending, not like a pause.
    #[test]
    fn disabling_weather_dries_the_world_out() {
        let weather = raining();
        let wet = run(&weather, 60.0);
        let off = WeatherComponent {
            enabled: false,
            ..weather
        };
        let mut state = wet;
        for _ in 0..(120 * 60) {
            state = off.step(state, 1.0 / 60.0);
        }
        assert!(state.wet_diffuse < 0.35, "{}", state.wet_diffuse);
        assert_eq!(state.wind, [0.0, 0.0]);
    }

    #[test]
    fn a_preset_sets_the_weather_and_leaves_the_switch_alone() {
        let mut weather = WeatherComponent::default();
        assert!(weather.apply_preset("snow"));
        assert!(weather.temperature_c < 0.0);
        assert!(!weather.enabled);
        assert!(!weather.apply_preset("hail"));
    }

    /// One preset, both halves. Rain falling out of a clear sky is the
    /// failure this mapping exists to prevent.
    #[test]
    fn every_weather_preset_names_a_sky() {
        for (id, label, _) in PRESETS {
            let sky = sky_preset_for(id).unwrap_or_else(|| panic!("{label} names no sky"));
            let mut probe = crate::sky::SkyComponent::default();
            assert!(
                probe.apply_preset(sky),
                "{label} names sky {sky}, which does not exist"
            );
        }
        // A storm must actually close the sky, or the chain is decorative.
        let mut stormy = crate::sky::SkyComponent::default();
        stormy.apply_preset(sky_preset_for("storm").unwrap());
        assert!(stormy.coverage > 0.9, "a storm sky must be closed");
    }

    #[test]
    fn every_menu_preset_resolves_to_a_real_one() {
        for (id, label) in somnium_ui::commands::WEATHER_PRESETS {
            let mut weather = WeatherComponent::default();
            assert!(weather.apply_preset(id), "{label} ({id}) does not resolve");
        }
        assert_eq!(somnium_ui::commands::WEATHER_PRESETS.len(), PRESETS.len());
    }
}
