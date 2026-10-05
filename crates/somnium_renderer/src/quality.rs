//! Player-facing graphics presets.
//!
//! A preset is a *budget laid over the scene's authored settings*: every
//! field can only lower what a level asked for, never switch on something it
//! left off, so a scene that avoids an effect keeps avoiding it at every tier.
//! `Somnium` is the level exactly as authored; each tier below it drops the
//! features that cost the most GPU time in Town's measurements first
//! (ReSTIR GI ~6 ms, then scene resolution, GTAO, traced direct light and water).
//!
//! Chosen by `SOMNIUM_GRAPHICS=low|medium|high|somnium` at start, or at run
//! time through `SomniumRenderer::set_graphics_preset` (the settings menu).

/// The four tiers a player picks from.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum GraphicsPreset {
    /// Lowest cost: two-thirds-and-under resolution, no screen-space or traced
    /// lighting extras, nearer foliage and shadow distances.
    Low,
    /// Probe GI instead of traced GI, reduced resolution, GTAO kept.
    Medium,
    /// Everything but traced global illumination, which probes stand in for.
    High,
    /// The scene as authored. The development default.
    #[default]
    Somnium,
}

/// What a preset allows. Multipliers scale authored distances and resolution;
/// flags gate authored features (an unauthored feature stays off regardless).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GraphicsBudget {
    /// Upper bound on the scene's render scale (1.0 = as authored).
    pub render_scale: f32,
    /// Multiplier on foliage and prop draw distances, LOD hand-overs included.
    pub draw_distance: f32,
    /// Multiplier on foliage shadow distances.
    pub shadow_distance: f32,
    /// Screen-space ambient occlusion.
    pub gtao: bool,
    /// ReSTIR GI, the world radiance cache and traced specular GI. When an
    /// authored scene loses these, DDGI probes stand in (see `probes`).
    pub traced_gi: bool,
    /// ReSTIR direct lighting (traced shadows for many lights).
    pub traced_direct: bool,
    /// DDGI irradiance probes, as authored or as the stand-in for traced GI.
    pub probes: bool,
    /// Ray-traced water reflection and refraction.
    pub traced_water: bool,
    /// Percentage-closer soft shadows.
    pub soft_shadows: bool,
    /// Screen-space contact shadows.
    pub contact_shadows: bool,
    /// Volumetric light shafts (the fog itself is part of the art and stays).
    pub light_shafts: bool,
    /// Depth of field and motion blur.
    pub camera_effects: bool,
    /// Plants sway in the scene's wind (`wind.rs`). Nearly free on the GPU,
    /// but a still forest is the cheapest thing a low tier can give up.
    pub foliage_wind: bool,
}

impl GraphicsPreset {
    /// Every tier, lowest first, for a settings list.
    pub const ALL: [Self; 4] = [Self::Low, Self::Medium, Self::High, Self::Somnium];

    /// Display and config name.
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Self::Low => "Low",
            Self::Medium => "Medium",
            Self::High => "High",
            Self::Somnium => "Somnium",
        }
    }

    /// Parse a name case-insensitively; unknown names are `None`.
    #[must_use]
    pub fn from_name(name: &str) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|p| p.name().eq_ignore_ascii_case(name.trim()))
    }

    /// `SOMNIUM_GRAPHICS`, or the default.
    #[must_use]
    pub fn from_env() -> Self {
        std::env::var("SOMNIUM_GRAPHICS")
            .ok()
            .and_then(|v| Self::from_name(&v))
            .unwrap_or_default()
    }

    /// What this tier allows.
    #[must_use]
    pub fn budget(self) -> GraphicsBudget {
        let all = GraphicsBudget {
            render_scale: 1.0,
            draw_distance: 1.0,
            shadow_distance: 1.0,
            gtao: true,
            traced_gi: true,
            traced_direct: true,
            probes: true,
            traced_water: true,
            soft_shadows: true,
            contact_shadows: true,
            light_shafts: true,
            camera_effects: true,
            foliage_wind: true,
        };
        match self {
            Self::Somnium => all,
            Self::High => GraphicsBudget {
                traced_gi: false,
                ..all
            },
            Self::Medium => GraphicsBudget {
                render_scale: 0.67,
                draw_distance: 0.85,
                shadow_distance: 0.75,
                traced_gi: false,
                traced_direct: false,
                traced_water: false,
                soft_shadows: false,
                camera_effects: false,
                ..all
            },
            Self::Low => GraphicsBudget {
                render_scale: 0.55,
                draw_distance: 0.65,
                shadow_distance: 0.5,
                gtao: false,
                traced_gi: false,
                traced_direct: false,
                probes: true,
                traced_water: false,
                soft_shadows: false,
                contact_shadows: false,
                light_shafts: false,
                camera_effects: false,
                foliage_wind: false,
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tiers_only_ever_lower_the_budget_going_down() {
        let score = |b: GraphicsBudget| {
            b.render_scale
                + b.draw_distance
                + b.shadow_distance
                + [
                    b.gtao,
                    b.traced_gi,
                    b.traced_direct,
                    b.probes,
                    b.traced_water,
                    b.soft_shadows,
                    b.contact_shadows,
                    b.light_shafts,
                    b.camera_effects,
                    b.foliage_wind,
                ]
                .iter()
                .filter(|f| **f)
                .count() as f32
        };
        for pair in GraphicsPreset::ALL.windows(2) {
            assert!(
                score(pair[0].budget()) < score(pair[1].budget()),
                "{pair:?}"
            );
        }
        assert_eq!(
            GraphicsPreset::from_name(" somnium "),
            Some(GraphicsPreset::Somnium)
        );
        assert_eq!(GraphicsPreset::from_name("ultra"), None);
        assert_eq!(GraphicsPreset::default(), GraphicsPreset::Somnium);
    }
}
