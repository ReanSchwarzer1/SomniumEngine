//! Phase 11.5F: Scene save/load — `.somnium` JSON format.
//!
//! ## Format (version 1)
//! ```json
//! {
//!   "version": 1,
//!   "entities": [
//!     {
//!       "local_idx": 0,
//!       "name": "SunLight",
//!       "transform": { "translation": [0,0,0], "rotation": [0,0,0,1], "scale": [1,1,1] },
//!       "light": { "kind": "Directional", "color": [1,1,1], "intensity": 5.0 },
//!       "mesh_kind": null,
//!       "parent_local_idx": null
//!     }
//!   ]
//! }
//! ```
//!
//! `parent_local_idx` is the index into the `entities` array, not the ECS entity index.
//! Mesh entities carry a `mesh_kind` string ("Cube", "Sphere", "Plane", "Cylinder")
//! so geometry can be regenerated on load.
#![allow(missing_docs)]

use crate::{
    LightComponent, LightType, MeshKind, Name, Parent, TerrainComponent, Transform, WaterComponent,
};
use somnium_ecs::Entity;
use somnium_ecs::World;

// ─── Save ─────────────────────────────────────────────────────────────────

/// Serialize the current world to a `.somnium` JSON scene file.
///
/// Entities without a `Name` component are assigned a synthetic name.
/// `MeshComponent` GPU offsets are NOT saved; only the `MeshKind` tag is.
/// Parent-child relationships are preserved as `parent_local_idx` references.
pub fn save_scene(world: &World, path: &str) -> Result<(), String> {
    let entities: Vec<Entity> = world.entities().collect();

    // Map entity index → JSON array local index for parent references.
    let mut entity_to_local: std::collections::HashMap<u32, usize> = Default::default();
    for (i, &e) in entities.iter().enumerate() {
        entity_to_local.insert(e.index(), i);
    }

    let serial: Vec<serde_json::Value> = entities
        .iter()
        .enumerate()
        .map(|(local_idx, &entity)| {
            let name = world
                .get::<Name>(entity)
                .map(|n| n.as_str().to_string())
                .unwrap_or_else(|| format!("Entity_{}", entity.index()));

            let transform = world
                .get::<Transform>(entity)
                .copied()
                .unwrap_or_else(|| Transform::from_translation(glam::Vec3::ZERO));

            let light = world.get::<LightComponent>(entity).map(|lc| {
                serde_json::json!({
                    "kind": match lc.light_type {
                        LightType::Directional => "Directional",
                        LightType::Point       => "Point",
                        LightType::Spot        => "Spot",
                        LightType::Rect        => "Rect",
                        LightType::Disc        => "Disc",
                        LightType::Tube        => "Tube",
                    },
                    "color":       lc.color.to_array(),
                    "intensity":   lc.intensity,
                    "range":       lc.range,
                    "inner_angle": lc.inner_angle,
                    "outer_angle": lc.outer_angle,
                    "source_radius": lc.source_radius,
                    "area_width": lc.area_width,
                    "area_height": lc.area_height,
                })
            });

            let mesh_kind = world.get::<MeshKind>(entity).map(|mk| match mk {
                MeshKind::Cube => "Cube",
                MeshKind::Sphere => "Sphere",
                MeshKind::Plane => "Plane",
                MeshKind::Cylinder => "Cylinder",
            });

            let parent_local_idx = world
                .get::<Parent>(entity)
                .filter(|p| p.entity != Entity::DANGLING && world.is_alive(p.entity))
                .and_then(|p| entity_to_local.get(&p.entity.index()).copied());

            // Phase 14F-3: terrain config; heightmap/splat data live in the
            // sidecar binary written by app.rs (`<scene>.terrain<id>.bin`).
            let terrain = world.get::<TerrainComponent>(entity).map(|tc| {
                serde_json::json!({
                    "terrain_id":   tc.terrain_id,
                    "chunk_cells":  tc.chunk_cells,
                    "grid_size":    [tc.grid_x, tc.grid_z],
                    "cell_size":    tc.cell_size,
                    "height_scale": tc.height_scale,
                    "virtual_texturing": tc.virtual_texturing,
                    "virtual_texture_cache_mib": tc.virtual_texture_cache_mib,
                    "virtual_texture_uploads_per_frame": tc.virtual_texture_uploads_per_frame,
                })
            });

            let water = world.get::<WaterComponent>(entity).map(|water| {
                serde_json::json!({
                    "water_id": water.water_id,
                    "terrain_id": water.terrain_id,
                    "preset": water.preset,
                    "body_kind": water.body_kind,
                    "surface_level": water.surface_level,
                    "max_depth": water.max_depth,
                    "bounds": water.bounds,
                    "enabled": water.enabled,
                    "mask_asset": somnium_renderer::water_body::GREAT_LAKES_MASK,
                    "depth_asset": somnium_renderer::water_body::GREAT_LAKES_DEPTH,
                    "shore_sdf_asset": somnium_renderer::water_body::GREAT_LAKES_SHORE_SDF,
                    "deep_color": water.deep_color,
                    "shallow_color": water.shallow_color,
                    "edge_color": water.edge_color,
                    "clarity": water.clarity,
                    "edge_scale": water.edge_scale,
                    "amplitude": water.amplitude,
                    "coord_scale": water.coord_scale,
                    "coord_offset": water.coord_offset,
                    "wave_dir_a": water.wave_dir_a,
                    "wave_dir_b": water.wave_dir_b,
                    "wave_blend": water.wave_blend,
                    "wave_length_a": water.wave_length_a,
                    "wave_length_b": water.wave_length_b,
                    "wave_speed": water.wave_speed,
                    "wave_steepness": water.wave_steepness,
                    "absorption": water.absorption,
                    "scattering": water.scattering,
                    "roughness": water.roughness,
                    "anisotropy": water.anisotropy,
                    "ssr_strength": water.ssr_strength,
                    "rt_reflect_strength": water.rt_reflect_strength,
                    "reflect_debug": water.reflect_debug,
                    "spectrum_blend": water.spectrum_blend,
                    "wind_speed": water.wind_speed,
                    "foam_decay": water.foam_decay,
                    "foam_threshold": water.foam_threshold,
                    "caustic_strength": water.caustic_strength,
                    "underwater_enabled": water.underwater_enabled,
                })
            });

            serde_json::json!({
                "local_idx":        local_idx,
                "name":             name,
                "transform": {
                    "translation":  transform.translation.to_array(),
                    "rotation":     transform.rotation.to_array(),
                    "scale":        transform.scale.to_array(),
                },
                "light":            light,
                "mesh_kind":        mesh_kind,
                "terrain":          terrain,
                "water":            water,
                "parent_local_idx": parent_local_idx,
            })
        })
        .collect();

    let scene = serde_json::json!({ "version": 1, "entities": serial });
    let json = serde_json::to_string_pretty(&scene).map_err(|e| format!("Serialize error: {e}"))?;
    std::fs::write(path, json).map_err(|e| format!("Write error: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Children, WorldTransform};

    #[test]
    fn water_round_trip_preserves_assets_parameters_and_parent() {
        let mut world = World::new();
        let terrain = world.spawn((
            Transform::from_translation(glam::Vec3::ZERO),
            Name::new("Terrain"),
            WorldTransform::identity(),
            TerrainComponent {
                terrain_id: 4,
                chunk_cells: 64,
                grid_x: 16,
                grid_z: 16,
                cell_size: 1.0,
                height_scale: 1.0,
                virtual_texturing: true,
                virtual_texture_cache_mib: 96,
                virtual_texture_uploads_per_frame: 12,
                ..TerrainComponent::default()
            },
            Children::empty(),
        ));
        let water_component = WaterComponent::great_lakes(7, 4, [0.0, 0.0, 1024.0, 1024.0]);
        let water = world.spawn((
            Transform::from_translation(glam::Vec3::new(512.0, 15.0, 512.0)),
            Name::new("Water"),
            WorldTransform::identity(),
            water_component,
            Parent { entity: terrain },
        ));
        world.get_mut::<Children>(terrain).unwrap().push(water);
        let path = std::env::temp_dir().join("somnium_water_roundtrip.somnium");
        save_scene(&world, path.to_str().unwrap()).unwrap();
        let scene = parse_scene(path.to_str().unwrap()).unwrap();
        let entities = scene["entities"].as_array().unwrap();
        let terrain = entities.iter().find(|e| e["name"] == "Terrain").unwrap();
        assert_eq!(terrain["terrain"]["virtual_texturing"], true);
        assert_eq!(terrain["terrain"]["virtual_texture_cache_mib"], 96);
        assert_eq!(terrain["terrain"]["virtual_texture_uploads_per_frame"], 12);
        assert!(terrain["terrain"].get("virtual_texture_hits").is_none());
        let water = entities.iter().find(|e| e["name"] == "Water").unwrap();
        assert_eq!(water["parent_local_idx"], 0);
        assert_eq!(water["water"]["preset"], 1);
        let surface_level = water["water"]["surface_level"].as_f64().unwrap();
        assert!((surface_level - 16.1).abs() < 1.0e-5);
        let max_depth = water["water"]["max_depth"].as_f64().unwrap();
        assert!((max_depth - 18.6).abs() < 1.0e-5);
        assert_eq!(
            water["water"]["mask_asset"],
            somnium_renderer::water_body::GREAT_LAKES_MASK
        );
        assert_eq!(
            water["water"]["depth_asset"],
            somnium_renderer::water_body::GREAT_LAKES_DEPTH
        );
        assert_eq!(
            water["water"]["shore_sdf_asset"],
            somnium_renderer::water_body::GREAT_LAKES_SHORE_SDF
        );
    }
}

// ─── Parse ────────────────────────────────────────────────────────────────

/// Read and parse a `.somnium` scene file, returning the raw JSON value.
///
/// The caller (typically the engine-level IPC handler in `app.rs`) is
/// responsible for actually spawning entities from the parsed data, since
/// mesh reconstruction requires GPU access not available here.
pub fn parse_scene(path: &str) -> Result<serde_json::Value, String> {
    let content = std::fs::read_to_string(path).map_err(|e| format!("Read error: {e}"))?;
    let scene: serde_json::Value =
        serde_json::from_str(&content).map_err(|e| format!("JSON parse error: {e}"))?;
    let version = scene["version"].as_u64().unwrap_or(0);
    if version != 1 {
        return Err(format!("Unsupported scene version: {version}"));
    }
    Ok(scene)
}

// ─── Mesh kind helpers ────────────────────────────────────────────────────

/// Parse a mesh kind string from a scene file.
pub fn mesh_kind_from_str(s: &str) -> Option<MeshKind> {
    match s {
        "Cube" => Some(MeshKind::Cube),
        "Sphere" => Some(MeshKind::Sphere),
        "Plane" => Some(MeshKind::Plane),
        "Cylinder" => Some(MeshKind::Cylinder),
        _ => None,
    }
}
