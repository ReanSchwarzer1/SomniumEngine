//! Project-owned foliage kinds. Stable IDs 128..255 leave legacy palette IDs intact.
use serde::Deserialize;
use std::{
    collections::{BTreeMap, BTreeSet},
    path::PathBuf,
};

#[derive(Clone, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub(crate) struct BrushDefaults {
    pub single: bool,
    pub density: f32,
    pub layer: u8,
    pub min_layer_weight: f32,
    pub max_tilt_deg: f32,
    pub max_slope_deg: f32,
    pub scale_min: f32,
    pub scale_max: f32,
}
impl Default for BrushDefaults {
    fn default() -> Self {
        Self {
            single: false,
            density: 2.0,
            layer: 0,
            min_layer_weight: 0.0,
            max_tilt_deg: 0.0,
            max_slope_deg: 40.0,
            scale_min: 0.8,
            scale_max: 1.3,
        }
    }
}

#[derive(Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ProjectEntry {
    pub kind: u8,
    pub name: String,
    pub source: PathBuf,
    /// Flattened renderable-node ordinals, identical to ImportedMesh.node.
    pub primitives: Vec<u32>,
    /// Column-major adjustment before the source node transform; identity by default.
    #[serde(default)]
    pub local_transform: Option<[f32; 16]>,
    #[serde(default)]
    pub brush: BrushDefaults,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Document {
    version: u32,
    entries: Vec<ProjectEntry>,
}

fn parse(bytes: &[u8]) -> Result<BTreeMap<u8, ProjectEntry>, String> {
    let document: Document = serde_json::from_slice(bytes).map_err(|e| e.to_string())?;
    if document.version != 1 || document.entries.len() > 128 {
        return Err("unsupported or oversized foliage palette".into());
    }
    let mut entries = BTreeMap::new();
    for entry in document.entries {
        let b = &entry.brush;
        let scalars = [
            b.density,
            b.min_layer_weight,
            b.max_tilt_deg,
            b.max_slope_deg,
            b.scale_min,
            b.scale_max,
        ];
        if entry.kind < 128
            || entry.name.trim().is_empty()
            || entry.name.len() > 128
            || entry.primitives.is_empty()
            || entry.primitives.len() > 256
            || entry.primitives.iter().collect::<BTreeSet<_>>().len() != entry.primitives.len()
            || scalars.iter().any(|v| !v.is_finite())
            || !(0.0..=40.0).contains(&b.density)
            || b.layer >= 32
            || !(0.0..=1.0).contains(&b.min_layer_weight)
            || !(0.0..=90.0).contains(&b.max_tilt_deg)
            || !(0.0..=90.0).contains(&b.max_slope_deg)
            || b.scale_min <= 0.0
            || b.scale_max < b.scale_min
            || b.scale_max > 1000.0
        {
            return Err(format!("invalid foliage palette entry {}", entry.kind));
        }
        if let Some(values) = entry.local_transform {
            let matrix = glam::Mat4::from_cols_array(&values);
            if values.iter().any(|v| !v.is_finite())
                || !matrix.determinant().is_finite()
                || matrix.determinant().abs() < 1e-10
                || values[3] != 0.0
                || values[7] != 0.0
                || values[11] != 0.0
                || values[15] != 1.0
            {
                return Err(format!(
                    "foliage kind {} needs an invertible affine local_transform",
                    entry.kind
                ));
            }
        }
        if entries.insert(entry.kind, entry).is_some() {
            return Err("duplicate foliage kind".into());
        }
    }
    Ok(entries)
}

pub(crate) fn load(
    config: &crate::config::EngineConfig,
) -> Result<BTreeMap<u8, ProjectEntry>, String> {
    let Some(root) = config.project_root.as_deref() else {
        return Ok(BTreeMap::new());
    };
    let path = config.content_root.join("foliage.palette.json");
    if !path.exists() {
        return Ok(BTreeMap::new());
    }
    let project = crate::authoring::project::ProjectPaths::open(root)?;
    let mut entries = parse(&std::fs::read(path).map_err(|e| e.to_string())?)?;
    for entry in entries.values_mut() {
        entry.source = project.resolve(&entry.source)?;
        if !entry.source.is_file() {
            return Err(format!(
                "foliage source missing: {}",
                entry.source.display()
            ));
        }
    }
    Ok(entries)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn stable_project_ids_preserve_all_selected_material_parts_and_transform() {
        let bytes=br#"{"version":1,"entries":[{"kind":137,"name":"mixed cover","source":"assets/cover.gltf","primitives":[5,6,7,8,9],"local_transform":[1,0,0,0,0,1,0,0,0,0,1,0,-2,0,3,1]}]}"#;
        let parsed = parse(bytes).unwrap();
        let e = &parsed[&137];
        assert_eq!(e.primitives, [5, 6, 7, 8, 9]);
        assert_eq!(
            glam::Mat4::from_cols_array(&e.local_transform.unwrap())
                .transform_point3(glam::Vec3::ZERO),
            glam::Vec3::new(-2.0, 0.0, 3.0)
        );
        assert!(parse(br#"{"version":1,"entries":[{"kind":0,"name":"bad","source":"x","primitives":[0]}]}"#).is_err());
        assert!(parse(br#"{"version":1,"entries":[{"kind":128,"name":"bad","source":"x","primitives":[0,0]}]}"#).is_err());
    }
}
