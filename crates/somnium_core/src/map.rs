//! Version-2 map recipes (`assets/Maps/*.somnium`).
//!
//! Distinct from the version-1 entity dump: a map file is a `kind` that a
//! factory rebuilds. Height and splat stay procedural so git does not hold
//! 1024² bins.

use crate::{
    CameraSettingsComponent, LightComponent, Name, Parent, Transform, WaterComponent,
    WorldTransform, create_default_landscape, create_island_landscape,
    landscape::DefaultLandscapePreset, look_rotation_neg_z,
};
use somnium_ecs::World;

/// Startup / drawer path for the coastal launch map.
pub const DEFAULT_MAP_PATH: &str = "assets/Maps/Coastal.somnium";

/// Factory kind stored in a version-2 `.somnium`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MapKind {
    /// Great Lakes height, 32-layer Appalachia, frozen water.
    Coastal,
    /// 512 m island with surrounding ocean, hero bank (layers 0–15) only.
    Island,
}

impl MapKind {
    fn from_str(kind: &str) -> Result<Self, String> {
        match kind {
            "coastal" => Ok(Self::Coastal),
            "island" => Ok(Self::Island),
            other => Err(format!("unknown map kind: {other}")),
        }
    }

    /// Parse a map name, case- and whitespace-insensitively.
    ///
    /// Public because Phase DOOM-A's timing runs have to select a map from the
    /// environment: the two recipes have deliberately different GPU budgets
    /// (Coastal publishes 32 layers, Island 16), so a measurement that does not
    /// say which one it ran on says nothing at all.
    pub fn parse(kind: &str) -> Result<Self, String> {
        Self::from_str(&kind.trim().to_ascii_lowercase())
    }
}

/// Camera / water handles the demo needs after a factory spawn.
pub struct MapLoadResult {
    /// Which factory ran.
    pub kind: MapKind,
    /// Editor fly-cam seed.
    pub camera_position: glam::Vec3,
    /// Editor fly-cam yaw in degrees.
    pub camera_yaw_degrees: f32,
    /// Editor fly-cam pitch in degrees.
    pub camera_pitch_degrees: f32,
    /// Terrain GPU id, if the factory created one.
    pub terrain_id: Option<u32>,
    /// Terrain root translation.
    pub terrain_origin: glam::Vec3,
    /// Water body for the optional coastal boat.
    pub water: Option<WaterComponent>,
    /// Authoring recipe used for this load.
    pub preset: DefaultLandscapePreset,
}

/// Parse a version-2 map JSON object.
pub fn parse_map_kind_json(text: &str) -> Result<MapKind, String> {
    let value: serde_json::Value =
        serde_json::from_str(text).map_err(|e| format!("JSON parse error: {e}"))?;
    let version = value["version"].as_u64().unwrap_or(0);
    if version != 2 {
        return Err(format!("unsupported map version: {version} (expected 2)"));
    }
    let kind = value["kind"]
        .as_str()
        .ok_or_else(|| "map recipe missing kind".to_string())?;
    MapKind::from_str(kind)
}

/// Read a version-2 map file and return its factory kind.
pub fn parse_map_file(path: &str) -> Result<MapKind, String> {
    let content = std::fs::read_to_string(path).map_err(|e| format!("Read error: {e}"))?;
    parse_map_kind_json(&content)
}

fn camera_forward(yaw_degrees: f32, pitch_degrees: f32) -> glam::Vec3 {
    let yaw = yaw_degrees.to_radians();
    let pitch = pitch_degrees.to_radians();
    glam::Vec3::new(
        yaw.cos() * pitch.cos(),
        pitch.sin(),
        yaw.sin() * pitch.cos(),
    )
    .normalize()
}

fn spawn_sun_post_camera(world: &mut World, preset: &DefaultLandscapePreset) {
    let elevation = std::env::var("SOMNIUM_SUN_ELEVATION")
        .ok()
        .and_then(|v| v.parse::<f32>().ok())
        .unwrap_or(35.0);
    let azimuth = std::env::var("SOMNIUM_SUN_AZIMUTH")
        .ok()
        .and_then(|v| v.parse::<f32>().ok())
        .unwrap_or(-30.0);
    let light_rot = glam::Quat::from_euler(
        glam::EulerRot::YXZ,
        azimuth.to_radians(),
        -elevation.to_radians(),
        0.0,
    );
    world.spawn((
        Transform {
            translation: glam::Vec3::ZERO,
            rotation: light_rot,
            scale: glam::Vec3::ONE,
        },
        LightComponent::directional(crate::light_units::lux::DIRECT_SUNLIGHT),
        Name::new("SunLight"),
        WorldTransform::identity(),
    ));
    world.spawn((
        Transform::from_translation(glam::Vec3::ZERO),
        Name::new("Post Processing"),
        WorldTransform::identity(),
        preset.post_process.clone(),
    ));
    // CONTROL-L: every map gets a day cycle, disabled, at the hour the
    // env-var defaults already put the sun at. Disabled because a map opening
    // with its sun somewhere other than where it has always been would be the
    // retuning §3 forbids; enabling it in Details is one click, and the hour
    // is already right when you do.
    world.spawn((
        Transform::from_translation(glam::Vec3::ZERO),
        Name::new("Environment"),
        WorldTransform::identity(),
        crate::time_of_day::TimeOfDayComponent {
            enabled: false,
            hour: 14.0,
            ..crate::time_of_day::TimeOfDayComponent::default()
        },
        // CONTROL-M: the sky rides on the same entity, also off. §3 forbids
        // changing what an existing scene draws; a cloud layer that appeared
        // over Coastal on its own would be exactly that.
        crate::sky::SkyComponent::default(),
        // CONTROL-N: and the weather, also off. Same reasoning again.
        crate::weather::WeatherComponent::default(),
    ));
    world.spawn((
        Transform {
            translation: preset.camera_position,
            rotation: look_rotation_neg_z(camera_forward(
                preset.camera_yaw_degrees,
                preset.camera_pitch_degrees,
            )),
            scale: glam::Vec3::ONE,
        },
        Name::new("Camera"),
        WorldTransform::identity(),
        CameraSettingsComponent::from_env(),
    ));
}

fn spawn_landscape(
    world: &mut World,
    built: crate::BuiltLandscape,
) -> (DefaultLandscapePreset, Option<u32>, Option<WaterComponent>) {
    let preset = built.preset;
    let water_component = built.water.water;
    let terrain_id = built.terrain.terrain.as_ref().map(|t| t.terrain_id);
    let terrain = built.terrain.respawn(world);
    let mut water_snapshot = built.water;
    water_snapshot.parent = Some(Parent { entity: terrain });
    let water = water_snapshot.respawn(world);
    world
        .get_mut::<crate::Children>(terrain)
        .unwrap()
        .push(water);
    (preset, terrain_id, water_component)
}

/// Spawn a map factory into an existing world (startup: keep cubes / extras).
pub fn spawn_map(
    world: &mut World,
    renderer: &mut somnium_renderer::SomniumRenderer,
    render_ctx: &somnium_renderer::RenderContext,
    kind: MapKind,
) -> Result<MapLoadResult, String> {
    // DOOM-I: see `create_default_landscape` for why these spans are `info!`.
    let started = std::time::Instant::now();
    let built = match kind {
        MapKind::Coastal => create_default_landscape(renderer, render_ctx)?,
        MapKind::Island => create_island_landscape(renderer, render_ctx)?,
    };
    let landscape = started.elapsed();
    let (preset, terrain_id, water) = spawn_landscape(world, built);
    spawn_sun_post_camera(world, &preset);
    tracing::info!(
        landscape_ms = landscape.as_secs_f32() * 1000.0,
        spawn_ms = (started.elapsed() - landscape).as_secs_f32() * 1000.0,
        total_ms = started.elapsed().as_secs_f32() * 1000.0,
        ?kind,
        "Map spawned"
    );
    Ok(MapLoadResult {
        kind,
        camera_position: preset.camera_position,
        camera_yaw_degrees: preset.camera_yaw_degrees,
        camera_pitch_degrees: preset.camera_pitch_degrees,
        terrain_id,
        terrain_origin: preset.terrain_translation,
        water,
        preset,
    })
}

/// GPU reset, despawn every entity, then run the map factory.
pub fn load_map(
    world: &mut World,
    renderer: &mut somnium_renderer::SomniumRenderer,
    render_ctx: &somnium_renderer::RenderContext,
    path: &str,
) -> Result<MapLoadResult, String> {
    let kind = parse_map_file(path)?;
    renderer.wait_gpu(render_ctx);
    renderer.reset_scene_gpu();
    let all: Vec<somnium_ecs::Entity> = world.entities().collect();
    for e in all {
        world.despawn(e);
    }
    spawn_map(world, renderer, render_ctx, kind)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_version_2_coastal_and_island() {
        assert_eq!(
            parse_map_kind_json(r#"{ "version": 2, "kind": "coastal" }"#).unwrap(),
            MapKind::Coastal
        );
        assert_eq!(
            parse_map_kind_json(r#"{ "version": 2, "kind": "island" }"#).unwrap(),
            MapKind::Island
        );
    }

    #[test]
    fn parse_rejects_entity_dump_and_unknown_kind() {
        assert!(parse_map_kind_json(r#"{ "version": 1, "entities": [] }"#).is_err());
        assert!(parse_map_kind_json(r#"{ "version": 2, "kind": "moon" }"#).is_err());
    }

    #[test]
    fn shipped_map_files_parse() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let coastal = root.join("assets/Maps/Coastal.somnium");
        let island = root.join("assets/Maps/Island.somnium");
        assert_eq!(
            parse_map_file(coastal.to_str().unwrap()).unwrap(),
            MapKind::Coastal
        );
        assert_eq!(
            parse_map_file(island.to_str().unwrap()).unwrap(),
            MapKind::Island
        );
    }
}
