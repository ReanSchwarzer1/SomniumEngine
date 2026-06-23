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

use somnium_ecs::World;
use crate::{LightComponent, LightType, MeshKind, Name, Parent, TerrainComponent, Transform};
use somnium_ecs::Entity;

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
                    },
                    "color":       lc.color.to_array(),
                    "intensity":   lc.intensity,
                    "range":       lc.range,
                    "inner_angle": lc.inner_angle,
                    "outer_angle": lc.outer_angle,
                })
            });

            let mesh_kind = world.get::<MeshKind>(entity).map(|mk| match mk {
                MeshKind::Cube     => "Cube",
                MeshKind::Sphere   => "Sphere",
                MeshKind::Plane    => "Plane",
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
                "parent_local_idx": parent_local_idx,
            })
        })
        .collect();

    let scene = serde_json::json!({ "version": 1, "entities": serial });
    let json =
        serde_json::to_string_pretty(&scene).map_err(|e| format!("Serialize error: {e}"))?;
    std::fs::write(path, json).map_err(|e| format!("Write error: {e}"))
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
        "Cube"     => Some(MeshKind::Cube),
        "Sphere"   => Some(MeshKind::Sphere),
        "Plane"    => Some(MeshKind::Plane),
        "Cylinder" => Some(MeshKind::Cylinder),
        _ => None,
    }
}
