//! Versioned user brush preferences. Resources use built-in palette paths/names,
//! never runtime indices. Scene strokes remain in their existing authored stores.
use serde::{Deserialize, Serialize};
use somnium_renderer::terrain::{
    brush::{BrushMode, TerrainBrush},
    foliage_paint::FoliageBrush,
};

#[derive(Serialize, Deserialize)]
#[serde(default)]
pub(crate) struct BrushPreferences {
    version: u32,
    operation: String,
    terrain_layer: String,
    radius: f32,
    strength: f32,
    hardness: f32,
    foliage_resource: String,
    foliage_radius: f32,
    density: f32,
    slope: f32,
    scale_min: f32,
    scale_max: f32,
    min_layer_weight: f32,
    single: bool,
    erase: bool,
}
impl Default for BrushPreferences {
    fn default() -> Self {
        Self::capture(&TerrainBrush::default(), &FoliageBrush::default(), false)
    }
}
impl BrushPreferences {
    fn path() -> Option<std::path::PathBuf> {
        std::env::var_os("APPDATA")
            .map(|p| std::path::PathBuf::from(p).join("SomniumEngine/authoring_tools.json"))
    }
    pub fn load() -> Self {
        Self::path()
            .and_then(|p| std::fs::read(p).ok())
            .and_then(|b| serde_json::from_slice::<Self>(&b).ok())
            .filter(|p| p.version == 1)
            .unwrap_or_default()
    }
    pub fn capture(terrain: &TerrainBrush, foliage: &FoliageBrush, erase: bool) -> Self {
        Self {
            version: 1,
            operation: terrain.mode.label().into(),
            terrain_layer: somnium_renderer::terrain::textures::LAYER_NAMES
                .get(terrain.paint_layer)
                .unwrap_or(&"")
                .to_string(),
            radius: terrain.radius,
            strength: terrain.strength,
            hardness: terrain.hardness,
            foliage_resource: crate::app::FOLIAGE_PALETTE
                .get(foliage.kind as usize)
                .unwrap_or(&crate::app::FOLIAGE_PALETTE[0])
                .path
                .into(),
            foliage_radius: foliage.radius,
            density: foliage.density,
            slope: foliage.max_slope_deg,
            scale_min: foliage.scale_min,
            scale_max: foliage.scale_max,
            min_layer_weight: foliage.min_layer_weight,
            single: foliage.single,
            erase,
        }
    }
    pub fn apply(&self, terrain: &mut TerrainBrush, foliage: &mut FoliageBrush) -> bool {
        let finite = |value: f32, default: f32, lo: f32, hi: f32| {
            if value.is_finite() {
                value.clamp(lo, hi)
            } else {
                default
            }
        };
        terrain.mode = [
            BrushMode::Raise,
            BrushMode::Lower,
            BrushMode::Smooth,
            BrushMode::Flatten,
            BrushMode::Noise,
            BrushMode::Paint,
        ]
        .into_iter()
        .find(|mode| mode.label() == self.operation)
        .unwrap_or(BrushMode::Raise);
        terrain.paint_layer = somnium_renderer::terrain::textures::LAYER_NAMES
            .iter()
            .position(|name| *name == self.terrain_layer)
            .unwrap_or(0);
        terrain.radius = finite(self.radius, 8.0, 0.5, 128.0);
        terrain.strength = finite(self.strength, 0.5, 0.05, 1.0);
        terrain.hardness = finite(self.hardness, 0.3, 0.0, 1.0);
        foliage.kind = crate::app::FOLIAGE_PALETTE
            .iter()
            .position(|entry| entry.path == self.foliage_resource)
            .unwrap_or(0) as u8;
        let entry = &crate::app::FOLIAGE_PALETTE[foliage.kind as usize];
        foliage.layer = entry.layer;
        foliage.max_tilt_deg = entry.max_tilt_deg;
        foliage.radius = finite(self.foliage_radius, 6.0, 0.25, 200.0);
        foliage.density = finite(self.density, 2.0, 0.0, 40.0);
        foliage.max_slope_deg = finite(self.slope, 40.0, 0.0, 90.0);
        foliage.scale_min = finite(self.scale_min, 0.8, 0.01, 1000.0);
        foliage.scale_max = finite(self.scale_max, 1.3, foliage.scale_min, 1000.0);
        foliage.min_layer_weight = finite(self.min_layer_weight, 0.0, 0.0, 1.0);
        foliage.single = self.single;
        self.erase
    }
    pub fn save(&self) {
        if cfg!(test) {
            return;
        }
        let Some(path) = Self::path() else {
            return;
        };
        let result = (|| -> Result<(), Box<dyn std::error::Error>> {
            std::fs::create_dir_all(path.parent().unwrap())?;
            std::fs::write(path, serde_json::to_vec_pretty(self)?)?;
            Ok(())
        })();
        if let Err(error) = result {
            tracing::warn!(%error, "Could not save authoring tool preferences");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn brush_resources_and_options_survive_serialization() {
        let mut terrain = TerrainBrush::default();
        terrain.paint_layer = 3;
        terrain.mode = BrushMode::Paint;
        terrain.radius = 24.0;
        let mut foliage = FoliageBrush::default();
        foliage.kind = 4;
        foliage.density = 7.0;
        let saved = BrushPreferences::capture(&terrain, &foliage, true);
        let decoded: BrushPreferences =
            serde_json::from_slice(&serde_json::to_vec(&saved).unwrap()).unwrap();
        let mut t = TerrainBrush::default();
        let mut f = FoliageBrush::default();
        assert!(decoded.apply(&mut t, &mut f));
        assert_eq!(t.paint_layer, 3);
        assert_eq!(t.mode, BrushMode::Paint);
        assert_eq!(t.radius, 24.0);
        assert_eq!(f.kind, 4);
        assert_eq!(f.density, 7.0);
    }
    #[test]
    fn unavailable_resources_and_invalid_values_recover_safely() {
        let mut saved = BrushPreferences::default();
        saved.foliage_resource = "removed.glb".into();
        saved.terrain_layer = "removed".into();
        saved.radius = f32::NAN;
        saved.scale_min = 8.0;
        saved.scale_max = -1.0;
        let mut t = TerrainBrush::default();
        let mut f = FoliageBrush::default();
        saved.apply(&mut t, &mut f);
        assert_eq!(t.radius, 8.0);
        assert_eq!(t.paint_layer, 0);
        assert_eq!(f.kind, 0);
        assert!(f.scale_max >= f.scale_min);
    }
}
