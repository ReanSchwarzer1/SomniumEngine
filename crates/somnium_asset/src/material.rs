//! Native editable material assets.
//!
//! `.sommat` is the authored source of truth. GPU pool indices and texture
//! bindless indices are deliberately absent: those are reconstructed by the
//! renderer from durable [`AssetId`] references.

use std::{fs, io::Cursor, path::Path};

use base64::{engine::general_purpose::STANDARD, Engine as _};
use serde::{Deserialize, Serialize};
use somnium_ecs::reflect::{ComponentSchema, FieldType, ReflectError, ReflectField, ReflectValue};
use somnium_ecs::{component_schema, Component};

use crate::{
    database::{AssetId, ASSET_KIND_TEXTURE},
    AlphaMode,
};

/// Current on-disk `.sommat` header version.
pub const MATERIAL_ASSET_VERSION: u32 = 1;

/// File metadata kept out of the generated property rows.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MaterialHeader {
    #[serde(default = "current_version")]
    pub version: u32,
    /// A self-contained 64x64 PNG, refreshed when the asset is saved.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub preview_png_base64: Option<String>,
}

impl Default for MaterialHeader {
    fn default() -> Self {
        Self {
            version: MATERIAL_ASSET_VERSION,
            preview_png_base64: None,
        }
    }
}

const fn current_version() -> u32 {
    MATERIAL_ASSET_VERSION
}

/// Linear RGB explicitly distinguished from arbitrary three-vectors so the
/// schema chooses a colour editor.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct LinearColor(pub [f32; 3]);

impl Default for LinearColor {
    fn default() -> Self {
        Self([1.0; 3])
    }
}

impl ReflectField for LinearColor {
    fn field_type() -> FieldType {
        FieldType::Color
    }

    fn to_reflect(&self) -> ReflectValue {
        ReflectValue::Vec3(self.0)
    }

    fn from_reflect(value: &ReflectValue, field: &'static str) -> Result<Self, ReflectError> {
        match value {
            ReflectValue::Vec3(value) => Ok(Self(*value)),
            other => Err(ReflectError::TypeMismatch {
                field,
                expected: "color".into(),
                found: other.kind(),
            }),
        }
    }
}

impl ReflectField for AlphaMode {
    fn field_type() -> FieldType {
        FieldType::Enum(&["Opaque", "Mask", "Blend"])
    }

    fn to_reflect(&self) -> ReflectValue {
        ReflectValue::I64(match self {
            Self::Opaque => 0,
            Self::Mask => 1,
            Self::Blend => 2,
        })
    }

    fn from_reflect(value: &ReflectValue, field: &'static str) -> Result<Self, ReflectError> {
        match value {
            ReflectValue::I64(0) => Ok(Self::Opaque),
            ReflectValue::I64(1) => Ok(Self::Mask),
            ReflectValue::I64(2) => Ok(Self::Blend),
            other => Err(ReflectError::TypeMismatch {
                field,
                expected: "enum".into(),
                found: other.kind(),
            }),
        }
    }
}

/// One editable material. This exact type is serialized, reflected, previewed,
/// and converted to the GPU representation; there is no parallel editor DTO.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct MaterialAsset {
    pub header: MaterialHeader,
    pub base_color: LinearColor,
    pub opacity: f32,
    pub metallic: f32,
    pub roughness: f32,
    pub emissive: LinearColor,
    pub emissive_intensity: f32,
    pub transmission: f32,
    /// Phase CONTROL-N: how much water this surface takes up, `0..1`.
    ///
    /// Lagarde's porosity. Zero is sealed — glass, painted metal, a puddle's
    /// own surface — and one is bare concrete. It is the reason rain reads
    /// differently on a pavement and on a car parked on it, and it is a
    /// *material* property rather than a weather one because it does not
    /// change when the rain stops.
    pub porosity: f32,
    pub alpha_mode: AlphaMode,
    pub alpha_cutoff: f32,
    pub double_sided: bool,
    pub foliage: bool,
    /// Flat cut-out cards whose `uv.x` runs across the blade (Phase TSUSHIMA-J).
    ///
    /// Only this turns on the curved-card normal in `shading.wgsl`. See
    /// [`somnium_asset::LoadedMaterial::foliage_card`] for why it is authored
    /// rather than inferred, and what happened while it was inferred.
    pub foliage_card: bool,
    pub albedo_map: AssetId,
    pub normal_map: AssetId,
    pub metallic_roughness_map: AssetId,
    pub occlusion_map: AssetId,
    pub emissive_map: AssetId,
}

impl Default for MaterialAsset {
    fn default() -> Self {
        Self {
            header: MaterialHeader {
                version: MATERIAL_ASSET_VERSION,
                preview_png_base64: None,
            },
            base_color: LinearColor::default(),
            opacity: 1.0,
            metallic: 0.0,
            roughness: 0.5,
            emissive: LinearColor([0.0; 3]),
            emissive_intensity: 1.0,
            transmission: 0.0,
            // Half-porous by default: most authored surfaces are neither
            // sealed nor bare, and a default of zero would make the whole
            // feature invisible until somebody found the slider.
            porosity: 0.5,
            alpha_mode: AlphaMode::Opaque,
            alpha_cutoff: 0.5,
            double_sided: false,
            foliage: false,
            foliage_card: false,
            albedo_map: AssetId::NONE,
            normal_map: AssetId::NONE,
            metallic_roughness_map: AssetId::NONE,
            occlusion_map: AssetId::NONE,
            emissive_map: AssetId::NONE,
        }
    }
}

impl Component for MaterialAsset {}

/// The schema used by generated Details and generic reflected undo.
#[must_use]
pub fn material_asset_schema() -> ComponentSchema {
    component_schema! {
        MaterialAsset as "somnium.asset.Material", display "Material", version 1,
        fields {
            base_color { group: "Surface" },
            opacity { min: 0.0, max: 1.0, step: 0.01, group: "Surface" },
            metallic { min: 0.0, max: 1.0, step: 0.01, group: "Surface" },
            roughness { min: 0.0, max: 1.0, step: 0.01, group: "Surface" },
            emissive { group: "Emission" },
            emissive_intensity { min: 0.0, step: 0.1, group: "Emission" },
            porosity { min: 0.0, max: 1.0, step: 0.01, precision: 2, group: "Surface",
                doc: "How much water this surface takes up when it rains." },
            transmission { min: 0.0, max: 1.0, step: 0.01, group: "Transmission" },
            alpha_mode { group: "Raster" },
            alpha_cutoff { min: 0.0, max: 1.0, step: 0.01, group: "Raster" },
            double_sided { group: "Raster" },
            foliage { group: "Raster", advanced: true,
                doc: "Vegetation: two-sided, translucent, and floored away from a wet-metal sheen." },
            foliage_card { group: "Raster", advanced: true,
                doc: "Flat cut-out cards only. Bends normals across the card using uv.x, which is wrong for modelled or atlased plants." },
            albedo_map { group: "Textures", asset_kind_mask: ASSET_KIND_TEXTURE },
            normal_map { group: "Textures", asset_kind_mask: ASSET_KIND_TEXTURE },
            metallic_roughness_map { group: "Textures", asset_kind_mask: ASSET_KIND_TEXTURE },
            occlusion_map { group: "Textures", asset_kind_mask: ASSET_KIND_TEXTURE },
            emissive_map { group: "Textures", asset_kind_mask: ASSET_KIND_TEXTURE },
        }
    }
}

impl MaterialAsset {
    /// Decode the header thumbnail, if present and valid.
    #[must_use]
    pub fn preview_png(&self) -> Option<Vec<u8>> {
        STANDARD
            .decode(self.header.preview_png_base64.as_deref()?)
            .ok()
    }

    /// Replace the header thumbnail with a PNG encoded from a 64x64 RGBA cell.
    pub fn set_preview_rgba(&mut self, rgba: &[u8]) -> Result<(), String> {
        let image = image::RgbaImage::from_raw(64, 64, rgba.to_vec())
            .ok_or_else(|| "material preview must be a 64x64 RGBA cell".to_string())?;
        let mut png = Vec::new();
        image::DynamicImage::ImageRgba8(image)
            .write_to(&mut Cursor::new(&mut png), image::ImageFormat::Png)
            .map_err(|error| error.to_string())?;
        self.header.version = MATERIAL_ASSET_VERSION;
        self.header.preview_png_base64 = Some(STANDARD.encode(png));
        Ok(())
    }
}

/// Read and validate a `.sommat` document.
pub fn load_material(path: impl AsRef<Path>) -> Result<MaterialAsset, String> {
    let path = path.as_ref();
    let text = fs::read_to_string(path).map_err(|error| format!("{}: {error}", path.display()))?;
    let asset: MaterialAsset = serde_json::from_str(&text)
        .map_err(|error| format!("{}: invalid material: {error}", path.display()))?;
    if asset.header.version != MATERIAL_ASSET_VERSION {
        return Err(format!(
            "{}: unsupported material version {} (current {})",
            path.display(),
            asset.header.version,
            MATERIAL_ASSET_VERSION
        ));
    }
    Ok(asset)
}

/// Save a document, refreshing its self-contained preview first.
pub fn save_material(
    path: impl AsRef<Path>,
    asset: &mut MaterialAsset,
    preview_rgba: &[u8],
) -> Result<(), String> {
    let path = path.as_ref();
    asset.set_preview_rgba(preview_rgba)?;
    let text = serde_json::to_string_pretty(asset).map_err(|error| error.to_string())?;
    fs::write(path, format!("{text}\n")).map_err(|error| format!("{}: {error}", path.display()))
}

/// Create a new material without ever overwriting an existing asset.
pub fn create_material(path: impl AsRef<Path>) -> Result<MaterialAsset, String> {
    let path = path.as_ref();
    if path.exists() {
        return Err(format!("{} already exists", path.display()));
    }
    let mut asset = MaterialAsset::default();
    let preview = crate::preview::render_material_sphere(&asset);
    save_material(path, &mut asset, &preview)?;
    Ok(asset)
}

fn safe_stem(name: &str) -> String {
    let cleaned: String = name
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() || matches!(character, '-' | '_') {
                character
            } else {
                '_'
            }
        })
        .collect();
    let cleaned = cleaned.trim_matches('_');
    if cleaned.is_empty() {
        "Material".to_string()
    } else {
        cleaned.to_string()
    }
}

/// Choose a numeric sibling without overwriting. Used by import, duplicate,
/// and Make Unique so all three flows share the same naming rule.
#[must_use]
pub fn unique_sibling(path: impl AsRef<Path>) -> std::path::PathBuf {
    let path = path.as_ref();
    if !path.exists() {
        return path.to_path_buf();
    }
    let stem = path.file_stem().and_then(|s| s.to_str()).unwrap_or("Asset");
    let extension = path.extension().and_then(|s| s.to_str());
    for suffix in 1_u32.. {
        let mut candidate = path.with_file_name(format!("{stem}_{suffix}"));
        if let Some(extension) = extension {
            candidate.set_extension(extension);
        }
        if !candidate.exists() {
            return candidate;
        }
    }
    unreachable!()
}

/// Materialize glTF textures and editable `.sommat` siblings on the import
/// worker. Returns one material AssetId per `LoadedScene.materials` entry.
pub fn materialize_gltf_assets(
    scene: &crate::LoadedScene,
    source: impl AsRef<Path>,
    content_root: impl AsRef<Path>,
) -> Result<Vec<AssetId>, String> {
    let source = source.as_ref();
    let content_root = content_root.as_ref();
    let source_stem = safe_stem(
        source
            .file_stem()
            .and_then(|name| name.to_str())
            .unwrap_or("Imported"),
    );
    let output = if source.starts_with(content_root) {
        source.parent().unwrap_or(content_root).to_path_buf()
    } else {
        content_root.join("Imported").join(&source_stem)
    };
    fs::create_dir_all(&output).map_err(|error| format!("{}: {error}", output.display()))?;

    let mut texture_ids = Vec::with_capacity(scene.textures.len());
    for (index, texture) in scene.textures.iter().enumerate() {
        let path = unique_sibling(output.join(format!("{source_stem}_Texture_{index}.png")));
        image::save_buffer(
            &path,
            &texture.data,
            texture.width,
            texture.height,
            image::ColorType::Rgba8,
        )
        .map_err(|error| format!("{}: {error}", path.display()))?;
        let relative = path.strip_prefix(content_root).unwrap_or(&path);
        texture_ids.push(AssetId::from_relative_path(relative));
    }

    let texture = |index: Option<usize>| {
        index
            .and_then(|index| texture_ids.get(index).copied())
            .unwrap_or(AssetId::NONE)
    };
    let mut material_ids = Vec::with_capacity(scene.materials.len());
    for (index, source_material) in scene.materials.iter().enumerate() {
        let name = safe_stem(&source_material.name);
        let path = unique_sibling(output.join(format!("{source_stem}_{index}_{name}.sommat")));
        let mut asset = MaterialAsset {
            base_color: LinearColor([
                source_material.base_color[0],
                source_material.base_color[1],
                source_material.base_color[2],
            ]),
            opacity: source_material.base_color[3],
            metallic: source_material.metallic,
            roughness: source_material.roughness,
            emissive: LinearColor(source_material.emissive),
            emissive_intensity: source_material.emissive_intensity,
            transmission: source_material.transmission,
            alpha_mode: source_material.alpha_mode,
            alpha_cutoff: source_material.alpha_cutoff,
            double_sided: source_material.double_sided,
            foliage: source_material.foliage,
            foliage_card: source_material.foliage_card,
            albedo_map: texture(source_material.albedo_map),
            normal_map: texture(source_material.normal_map),
            metallic_roughness_map: texture(source_material.metallic_roughness_map),
            occlusion_map: texture(source_material.occlusion_map),
            emissive_map: texture(source_material.emissive_map),
            ..Default::default()
        };
        let preview = crate::preview::render_material_sphere(&asset);
        save_material(&path, &mut asset, &preview)?;
        let relative = path.strip_prefix(content_root).unwrap_or(&path);
        material_ids.push(AssetId::from_relative_path(relative));
    }
    Ok(material_ids)
}

/// Worker-side texture decode for a material slot.
pub fn load_material_texture(path: impl AsRef<Path>) -> Result<crate::LoadedTexture, String> {
    let path = path.as_ref();
    let image = image::open(path)
        .map_err(|error| format!("{}: {error}", path.display()))?
        .into_rgba8();
    Ok(crate::LoadedTexture {
        width: image.width(),
        height: image.height(),
        data: image.into_raw(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use somnium_ecs::reflect::ReflectValue;
    use std::sync::atomic::{AtomicU64, Ordering};

    static NEXT: AtomicU64 = AtomicU64::new(1);

    fn temp(name: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!(
            "somnium-material-{}-{name}.sommat",
            NEXT.fetch_add(1, Ordering::Relaxed)
        ))
    }

    #[test]
    fn every_gpu_semantic_and_five_texture_slots_round_trip() {
        let path = temp("roundtrip");
        let mut material = MaterialAsset {
            base_color: LinearColor([0.3, 0.4, 0.5]),
            opacity: 0.75,
            metallic: 1.0,
            roughness: 0.2,
            emissive: LinearColor([0.1, 0.2, 0.3]),
            emissive_intensity: 12.0,
            transmission: 0.4,
            alpha_mode: AlphaMode::Mask,
            alpha_cutoff: 0.35,
            double_sided: true,
            foliage: true,
            foliage_card: true,
            albedo_map: AssetId::from_raw(1),
            normal_map: AssetId::from_raw(2),
            metallic_roughness_map: AssetId::from_raw(3),
            occlusion_map: AssetId::from_raw(4),
            emissive_map: AssetId::from_raw(5),
            ..Default::default()
        };
        let preview = crate::preview::render_material_sphere(&material);
        save_material(&path, &mut material, &preview).unwrap();
        assert_eq!(load_material(&path).unwrap(), material);
        assert!(material
            .preview_png()
            .is_some_and(|png| png.starts_with(b"\x89PNG")));
        let _ = fs::remove_file(path);
    }

    #[test]
    fn schema_is_complete_and_texture_slots_reject_non_textures() {
        let schema = material_asset_schema();
        assert_eq!(schema.fields.len(), 18);
        let texture_fields: Vec<_> = schema
            .fields
            .iter()
            .filter(|field| field.ty == FieldType::Asset)
            .collect();
        assert_eq!(texture_fields.len(), 5);
        assert!(texture_fields
            .iter()
            .all(|field| field.asset_kind_mask == ASSET_KIND_TEXTURE));
        assert_eq!(
            schema.field_by_name("base_color").unwrap().ty,
            FieldType::Color
        );
    }

    #[test]
    fn reflected_roughness_and_metallic_make_a_polished_material() {
        let schema = material_asset_schema();
        let mut world = somnium_ecs::World::new();
        let entity = world.spawn((MaterialAsset::default(),));
        let mut patch = somnium_ecs::reflect::ReflectObject::new();
        patch.insert(
            schema.field_by_name("roughness").unwrap().id,
            ReflectValue::F64(0.2),
        );
        patch.insert(
            schema.field_by_name("metallic").unwrap().id,
            ReflectValue::F64(1.0),
        );
        (schema.apply)(&mut world, entity, &patch).unwrap();
        let material = world.get::<MaterialAsset>(entity).unwrap();
        assert_eq!((material.roughness, material.metallic), (0.2, 1.0));
    }

    #[test]
    fn create_refuses_to_overwrite() {
        let path = temp("no-overwrite");
        fs::write(&path, "mine").unwrap();
        assert!(create_material(&path).is_err());
        assert_eq!(fs::read_to_string(&path).unwrap(), "mine");
        let _ = fs::remove_file(path);
    }

    #[test]
    fn gltf_import_writes_editable_material_and_embedded_texture_siblings() {
        let root = std::env::temp_dir().join(format!(
            "somnium-gltf-materials-{}",
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(&root).unwrap();
        let scene = crate::LoadedScene {
            textures: vec![crate::LoadedTexture {
                data: vec![255, 0, 0, 255],
                width: 1,
                height: 1,
            }],
            materials: vec![crate::LoadedMaterial {
                name: "Polished Steel".into(),
                base_color: [0.7, 0.75, 0.8, 1.0],
                roughness: 0.2,
                metallic: 1.0,
                albedo_map: Some(0),
                occlusion_map: None,
                normal_map: None,
                metallic_roughness_map: None,
                alpha_mode: AlphaMode::Opaque,
                alpha_cutoff: 0.5,
                transmission: 0.0,
                foliage: false,
                foliage_card: false,
                emissive: [0.0; 3],
                emissive_intensity: 1.0,
                emissive_map: None,
                double_sided: false,
            }],
            ..Default::default()
        };
        let ids = materialize_gltf_assets(&scene, "C:/outside/model.glb", &root).unwrap();
        assert_eq!(ids.len(), 1);
        let sommat = fs::read_dir(root.join("Imported/model"))
            .unwrap()
            .filter_map(Result::ok)
            .map(|entry| entry.path())
            .find(|path| path.extension().is_some_and(|e| e == "sommat"))
            .unwrap();
        let material = load_material(sommat).unwrap();
        assert_eq!((material.roughness, material.metallic), (0.2, 1.0));
        assert_ne!(material.albedo_map, AssetId::NONE);
        assert_eq!(
            fs::read_dir(root.join("Imported/model"))
                .unwrap()
                .filter_map(Result::ok)
                .filter(|entry| entry.path().extension().is_some_and(|e| e == "png"))
                .count(),
            1
        );
        let _ = fs::remove_dir_all(root);
    }
}
