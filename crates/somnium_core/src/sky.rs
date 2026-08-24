//! Phase CONTROL-M: the sky's authoring surface.
//!
//! # This module is the phase's own test
//!
//! §1 of the phase document says the rendering half exists to prove the
//! reachability rule rather than to decorate it: *"If clouds ship with a
//! weather-map painter and a preset list built from a schema block, the thesis
//! held. If they ship with three more `SOMNIUM_*` variables, it did not."*
//!
//! So there is no cloud tuning anywhere else. Every number the march reads is
//! a field below, every field is in one `component_schema!` block, and the
//! renderer's [`CloudSettings`](somnium_renderer::pass::clouds::CloudSettings)
//! is populated from this component once per frame and from nowhere else.
//!
//! # The one thing that is not a field
//!
//! Presets. They are commands, for the same reason CONTROL-L's times are: a
//! preset stored as a field is a second description of the values it set, and
//! it starts lying the moment anybody moves a slider.

use somnium_ecs::Component;

/// The scene's sky and cloud layer.
///
/// Lives on the Environment entity beside [`TimeOfDayComponent`], because
/// coverage is one end of the chain CONTROL-N completes: coverage drives
/// precipitation drives wetness.
///
/// [`TimeOfDayComponent`]: crate::time_of_day::TimeOfDayComponent
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SkyComponent {
    /// Draw the cloud layer at all.
    ///
    /// Default **off**, and it stays off until the `.somtime` row in the
    /// evidence folder says what the pass costs. The engine is GPU-bound and
    /// shading-dominated; §3 makes turning this on before the profiler speaks
    /// an explicit non-goal.
    pub enabled: bool,
    /// Fraction of the sky with cloud in it. `0` is clear, `1` is solid.
    pub coverage: f32,
    /// `0` stratus, `0.5` cumulus, `1` cumulonimbus. Selects the vertical
    /// density profile, which is what stops a cumulus having a flat top.
    pub cloud_type: f32,
    /// Metres from the ground to the bottom of the layer.
    pub altitude: f32,
    /// Layer thickness in metres.
    pub thickness: f32,
    /// Overall density multiplier. Zero skips the march entirely.
    pub density: f32,
    /// Wind velocity in metres per second, world XZ.
    ///
    /// CONTROL-N promotes this to the scene's one global wind vector, read by
    /// foliage sway, the ocean spectrum and precipitation shear as well.
    pub wind: [f32; 2],
    /// Strength of the high-frequency erosion that turns a smooth mass into
    /// billows. Costs a second 3-D fetch per lit sample.
    pub detail_strength: f32,
    /// How much extra light a precipitating column absorbs. Read by CONTROL-N
    /// as the rain driver; on its own it only darkens the cloud's base.
    pub precipitation: f32,
    /// Metres of world per repeat of the weather map. Larger is a bigger sky.
    pub weather_scale: f32,
    /// Metres of world per repeat of the base shape volume.
    pub shape_scale: f32,
    /// Ambient contribution from the sky's own multiscatter term.
    pub ambient: f32,
    /// Henyey–Greenstein forward lobe — the silver lining.
    pub phase_forward: f32,
    /// Henyey–Greenstein backward lobe. Negative.
    pub phase_backward: f32,
    /// Blend between the two lobes.
    pub phase_blend: f32,
    /// How much of the sun a cloud takes away from the ground, `0..1`.
    pub shadow_strength: f32,
    /// Half-extent of the ground shadow field in metres, centred on the
    /// camera. Larger covers more world at lower resolution.
    pub shadow_extent: f32,
    /// Primary march steps. The single biggest cost knob.
    pub max_steps: i32,
    /// Cone-sampled steps toward the sun per lit sample.
    pub light_steps: i32,
    /// Distance in metres at which the march gives up.
    pub max_distance: f32,
    /// Placement seed for the weather field. Changing it reshuffles the sky.
    pub seed: i32,
}

impl Component for SkyComponent {}

impl Default for SkyComponent {
    fn default() -> Self {
        Self {
            enabled: false,
            coverage: 0.45,
            cloud_type: 0.4,
            altitude: 1500.0,
            thickness: 2200.0,
            density: 1.0,
            wind: [12.0, 4.0],
            detail_strength: 0.6,
            precipitation: 0.0,
            weather_scale: 24_000.0,
            shape_scale: 6_000.0,
            ambient: 1.0,
            phase_forward: 0.8,
            phase_backward: -0.35,
            phase_blend: 0.4,
            shadow_strength: 0.7,
            shadow_extent: 4_000.0,
            max_steps: 48,
            light_steps: 6,
            max_distance: 60_000.0,
            seed: 1337,
        }
    }
}

impl SkyComponent {
    /// Convert to the renderer's settings.
    ///
    /// The one place the two vocabularies meet. `somnium_renderer` must not
    /// know what an ECS is and this component must not know what a bind group
    /// is, so exactly one function crosses the line and it is this one.
    #[must_use]
    pub fn to_settings(&self) -> somnium_renderer::pass::clouds::CloudSettings {
        somnium_renderer::pass::clouds::CloudSettings {
            coverage: self.coverage.clamp(0.0, 1.0),
            cloud_type: self.cloud_type.clamp(0.0, 1.0),
            altitude: self.altitude.max(1.0),
            thickness: self.thickness.max(1.0),
            density: self.density.max(0.0),
            wind: self.wind,
            detail_strength: self.detail_strength.clamp(0.0, 1.0),
            precipitation: self.precipitation.clamp(0.0, 1.0),
            weather_scale: self.weather_scale.max(1.0),
            shape_scale: self.shape_scale.max(1.0),
            ambient: self.ambient.max(0.0),
            phase_forward: self.phase_forward.clamp(0.0, 0.95),
            phase_backward: self.phase_backward.clamp(-0.95, 0.0),
            phase_blend: self.phase_blend.clamp(0.0, 1.0),
            shadow_strength: self.shadow_strength.clamp(0.0, 1.0),
            shadow_extent: self.shadow_extent.max(1.0),
            #[allow(clippy::cast_sign_loss)]
            max_steps: self.max_steps.clamp(8, 256) as u32,
            #[allow(clippy::cast_sign_loss)]
            light_steps: self.light_steps.clamp(1, 16) as u32,
            max_distance: self.max_distance.max(1.0),
            #[allow(clippy::cast_sign_loss)]
            seed: self.seed.max(0) as u32,
        }
    }

    /// Apply a named preset by id, returning false for an unknown one.
    ///
    /// A preset writes fields and then stops existing. `enabled` is
    /// deliberately untouched: choosing a look should not silently turn a pass
    /// on, and turning the pass on should not silently pick a look.
    pub fn apply_preset(&mut self, id: &str) -> bool {
        let Some((_, _, preset)) = PRESETS.iter().find(|(name, _, _)| *name == id) else {
            return false;
        };
        self.coverage = preset.coverage;
        self.cloud_type = preset.cloud_type;
        self.density = preset.density;
        self.precipitation = preset.precipitation;
        self.thickness = preset.thickness;
        self.wind = preset.wind;
        true
    }
}

/// The subset of fields a preset sets.
#[derive(Debug, Clone, Copy)]
pub struct SkyPreset {
    /// Fraction of the sky with cloud in it.
    pub coverage: f32,
    /// `0` stratus … `1` cumulonimbus.
    pub cloud_type: f32,
    /// Density multiplier.
    pub density: f32,
    /// Precipitation, which CONTROL-N reads as the rain driver.
    pub precipitation: f32,
    /// Layer thickness in metres — a storm is a deep cloud, not only a dense
    /// one, and a preset that changed only density would read as fog.
    pub thickness: f32,
    /// Wind in metres per second, world XZ.
    pub wind: [f32; 2],
}

/// The four skies CONTROL-M's evidence plan names as its captures.
///
/// They are the captures *because* they are the presets: a preset list and a
/// capture list that disagree is how a phase ends up with four screenshots of
/// states nobody can reach from the menu.
pub const PRESETS: [(&str, &str, SkyPreset); 4] = [
    (
        "clear",
        "Clear",
        SkyPreset {
            coverage: 0.08,
            cloud_type: 0.25,
            density: 0.7,
            precipitation: 0.0,
            thickness: 1400.0,
            wind: [8.0, 2.0],
        },
    ),
    (
        "scattered",
        "Scattered",
        SkyPreset {
            coverage: 0.40,
            cloud_type: 0.45,
            density: 1.0,
            precipitation: 0.0,
            thickness: 2200.0,
            wind: [12.0, 4.0],
        },
    ),
    (
        "overcast",
        "Overcast",
        SkyPreset {
            coverage: 0.92,
            cloud_type: 0.15,
            density: 1.3,
            precipitation: 0.15,
            thickness: 1800.0,
            wind: [16.0, 5.0],
        },
    ),
    (
        "storm",
        "Storm",
        SkyPreset {
            coverage: 1.0,
            cloud_type: 0.95,
            density: 2.0,
            precipitation: 1.0,
            thickness: 4200.0,
            wind: [34.0, 11.0],
        },
    ),
];

#[cfg(test)]
mod tests {
    use super::*;

    /// The pass ships off. Not a hedge — §3 makes turning it on before the
    /// profiler has spoken an explicit non-goal, and this is the line that
    /// would have to change to break that.
    #[test]
    fn clouds_are_off_until_the_number_exists() {
        assert!(!SkyComponent::default().enabled);
    }

    /// Everything the renderer reads is clamped on the way across, so an
    /// out-of-range authored value cannot reach a dispatch count or a division.
    #[test]
    fn the_conversion_clamps_every_value_the_renderer_divides_by() {
        let wild = SkyComponent {
            coverage: 5.0,
            cloud_type: -3.0,
            altitude: -100.0,
            thickness: 0.0,
            density: -1.0,
            detail_strength: 9.0,
            weather_scale: 0.0,
            shape_scale: -50.0,
            phase_forward: 4.0,
            phase_backward: 4.0,
            shadow_extent: 0.0,
            max_steps: 100_000,
            light_steps: -4,
            max_distance: 0.0,
            seed: -7,
            ..SkyComponent::default()
        };
        let s = wild.to_settings();
        assert_eq!(s.coverage, 1.0);
        assert_eq!(s.cloud_type, 0.0);
        assert!(s.altitude >= 1.0);
        assert!(s.thickness >= 1.0);
        assert_eq!(s.density, 0.0);
        assert_eq!(s.detail_strength, 1.0);
        assert!(s.weather_scale >= 1.0 && s.shape_scale >= 1.0);
        assert!(s.phase_forward <= 0.95);
        assert!(s.phase_backward <= 0.0);
        assert!(s.shadow_extent >= 1.0);
        assert_eq!(s.max_steps, 256);
        assert_eq!(s.light_steps, 1);
        assert!(s.max_distance >= 1.0);
        assert_eq!(s.seed, 0);
    }

    #[test]
    fn a_default_sky_survives_the_conversion_unchanged() {
        let s = SkyComponent::default().to_settings();
        let d = somnium_renderer::pass::clouds::CloudSettings::default();
        assert_eq!(s, d, "the two defaults must not drift apart");
    }

    /// A preset must change the look and must not change whether the pass is
    /// running: those are two decisions and conflating them is how a preset
    /// click costs someone 2 ms they did not ask for.
    #[test]
    fn a_preset_sets_the_look_and_leaves_the_switch_alone() {
        let mut sky = SkyComponent::default();
        assert!(sky.apply_preset("storm"));
        assert_eq!(sky.coverage, 1.0);
        assert_eq!(sky.precipitation, 1.0);
        assert!(!sky.enabled, "a preset must not turn the pass on");

        assert!(!sky.apply_preset("drizzle"), "unknown presets are refused");
    }

    /// Every preset the menu offers must be one the engine can apply.
    ///
    /// The two tables are deliberately split — ids and labels belong to the
    /// command registry, values belong here — and this is the seam that makes
    /// the split safe.
    #[test]
    fn every_menu_preset_resolves_to_a_real_one() {
        for (id, label) in somnium_ui::commands::SKY_PRESETS {
            let mut sky = SkyComponent::default();
            assert!(sky.apply_preset(id), "{label} ({id}) does not resolve");
        }
        assert_eq!(somnium_ui::commands::SKY_PRESETS.len(), PRESETS.len());
    }

    /// The four presets are the four captures. Ordered clear → storm, because
    /// the evidence folder's names are read in that order.
    #[test]
    fn the_presets_run_from_clear_to_storm() {
        let coverages: Vec<f32> = PRESETS.iter().map(|(_, _, p)| p.coverage).collect();
        for pair in coverages.windows(2) {
            assert!(pair[0] < pair[1], "presets must increase in coverage");
        }
        assert_eq!(PRESETS[0].0, "clear");
        assert_eq!(PRESETS[3].0, "storm");
    }
}
