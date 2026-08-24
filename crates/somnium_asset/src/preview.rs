//! Worker-side asset preview preparation and disk cache.

use crate::{
    database::{AssetKind, AssetRecord},
    load_gltf,
    material::{load_material, MaterialAsset},
    LoadedMesh, LoadedScene, Vertex,
};
use glam::{Mat4, Vec3, Vec4};
use image::{imageops::FilterType, DynamicImage, ImageBuffer, Rgba};
use std::{fs, path::Path};

pub const PREVIEW_CELL: u32 = 64;
const MESH_RENDER_SIZE: u32 = 96;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PreviewFrequency {
    Realtime,
    OnPropertyChange,
    OnAssetSave,
    Once,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PreparedPreview {
    pub rgba: Vec<u8>,
    pub from_disk_cache: bool,
}

/// One registered producer. Declining with `None` lets the next matching
/// producer run, then the drawer falls back to the grey kind icon and finally
/// its generic file icon.
#[derive(Clone, Copy)]
pub struct PreviewGenerator {
    /// Stable diagnostic name.
    pub name: &'static str,
    /// Asset kind accepted by this producer.
    pub kind: AssetKind,
    /// When an editor should invalidate this output.
    pub frequency: PreviewFrequency,
    generate: fn(&AssetRecord) -> Option<Vec<u8>>,
}

/// Ordered registry of preview generators.
#[derive(Clone, Default)]
pub struct PreviewGeneratorRegistry {
    generators: Vec<PreviewGenerator>,
}

impl PreviewGeneratorRegistry {
    /// Creates an empty registry for project/plugin generators.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Shipped registry. Expensive kinds are invalidated on save/property
    /// changes; immutable source assets render once per content hash.
    #[must_use]
    pub fn standard() -> Self {
        Self {
            generators: vec![
                PreviewGenerator {
                    name: "texture",
                    kind: AssetKind::Texture,
                    frequency: PreviewFrequency::Once,
                    generate: |r| decode_texture(&r.absolute_path),
                },
                PreviewGenerator {
                    name: "mesh-studio",
                    kind: AssetKind::Mesh,
                    frequency: PreviewFrequency::OnAssetSave,
                    generate: |r| render_mesh(&r.absolute_path),
                },
                PreviewGenerator {
                    name: "material",
                    kind: AssetKind::Material,
                    frequency: PreviewFrequency::OnPropertyChange,
                    generate: |r| decode_material(&r.absolute_path),
                },
                PreviewGenerator {
                    name: "scene",
                    kind: AssetKind::Scene,
                    frequency: PreviewFrequency::OnAssetSave,
                    generate: |r| decode_scene(&r.absolute_path),
                },
                PreviewGenerator {
                    name: "script-document",
                    kind: AssetKind::Script,
                    frequency: PreviewFrequency::Once,
                    generate: |_| Some(script_card()),
                },
            ],
        }
    }

    /// Registers a producer after existing entries of the same kind.
    pub fn register(&mut self, generator: PreviewGenerator) {
        self.generators.push(generator);
    }

    /// Registry metadata for diagnostics and invalidation wiring.
    #[must_use]
    pub fn entries(&self) -> &[PreviewGenerator] {
        &self.generators
    }

    fn generate(&self, record: &AssetRecord) -> Option<Vec<u8>> {
        self.generators
            .iter()
            .filter(|generator| generator.kind == record.kind)
            .find_map(|generator| (generator.generate)(record))
    }
}

/// CONTROL-J: a scene's thumbnail comes out of its own header.
///
/// Free, and never stale — it is written by the same operation that writes the
/// scene, so the two cannot disagree. It is also cheap: `read_header` stops
/// after a few hundred bytes rather than parsing a scene that may be
/// megabytes, which is the whole reason the container is framed.
///
/// A scene with no thumbnail — every one written before this phase — produces
/// no preview rather than a wrong one.
fn decode_scene(path: &Path) -> Option<Vec<u8>> {
    let header = crate::scene_file::read_header(path).ok()??;
    let png = header.thumbnail_png()?;
    let image = image::load_from_memory_with_format(&png, image::ImageFormat::Png).ok()?;
    Some(fit_to_cell(image))
}

fn decode_material(path: &Path) -> Option<Vec<u8>> {
    let material = load_material(path).ok()?;
    if let Some(png) = material.preview_png() {
        if let Ok(image) = image::load_from_memory_with_format(&png, image::ImageFormat::Png) {
            return Some(fit_to_cell(image));
        }
    }
    Some(render_material_sphere(&material))
}

/// Shared 64x64 material-studio sphere used by the drawer, save header, and
/// Details preview. It is deterministic and CPU-only so worker preparation
/// never touches the live renderer.
#[must_use]
pub fn render_material_sphere(material: &MaterialAsset) -> Vec<u8> {
    let mut cell = vec![0_u8; (PREVIEW_CELL * PREVIEW_CELL * 4) as usize];
    let base = Vec3::from(material.base_color.0);
    let emission = Vec3::from(material.emissive.0) * material.emissive_intensity;
    let light = Vec3::new(-0.45, 0.65, 0.62).normalize();
    let view = Vec3::new(0.0, 0.0, 1.0);
    for y in 0..PREVIEW_CELL {
        for x in 0..PREVIEW_CELL {
            let px = (x as f32 + 0.5) / PREVIEW_CELL as f32 * 2.0 - 1.0;
            let py = 1.0 - (y as f32 + 0.5) / PREVIEW_CELL as f32 * 2.0;
            let radius2 = px * px + py * py;
            if radius2 > 0.82 {
                continue;
            }
            let pz = (0.82 - radius2).sqrt();
            let normal = Vec3::new(px, py, pz).normalize();
            let diffuse = normal.dot(light).max(0.0);
            let half = (light + view).normalize();
            let gloss = (1.0 - material.roughness).clamp(0.0, 1.0);
            let specular = normal.dot(half).max(0.0).powf(2.0 + gloss * 126.0);
            let f0 = Vec3::splat(0.04).lerp(base, material.metallic.clamp(0.0, 1.0));
            let diffuse_color = base * (1.0 - material.metallic.clamp(0.0, 1.0));
            let color = diffuse_color * (0.08 + diffuse * 0.72)
                + f0 * specular * (0.35 + gloss * 1.8)
                + emission;
            let mapped = color / (Vec3::ONE + color);
            let srgb = mapped.powf(1.0 / 2.2) * 255.0;
            let offset = ((y * PREVIEW_CELL + x) * 4) as usize;
            cell[offset] = srgb.x.clamp(0.0, 255.0) as u8;
            cell[offset + 1] = srgb.y.clamp(0.0, 255.0) as u8;
            cell[offset + 2] = srgb.z.clamp(0.0, 255.0) as u8;
            cell[offset + 3] = (material.opacity.clamp(0.0, 1.0) * 255.0) as u8;
        }
    }
    cell
}

/// Produce a 64x64 RGBA preview without touching UI or renderer state.
///
/// Texture decode and the compact mesh studio render both run in a worker.
/// The UI-thread half is only the final 16 KiB atlas copy.
pub fn prepare_preview(
    record: &AssetRecord,
    cache_root: &Path,
) -> Result<Option<PreparedPreview>, String> {
    if record.kind == AssetKind::Folder {
        return Ok(None);
    }
    let hash = &record.metadata.content_hash;
    let cache_path = (!hash.is_empty()).then(|| cache_root.join(format!("{hash}.png")));
    if let Some(path) = cache_path.as_deref() {
        if let Some(rgba) = load_cell(path) {
            return Ok(Some(PreparedPreview {
                rgba,
                from_disk_cache: true,
            }));
        }
    }

    let rgba = PreviewGeneratorRegistry::standard().generate(record);
    let Some(rgba) = rgba else {
        return Ok(None);
    };
    if let Some(path) = cache_path.as_deref() {
        let _ = fs::create_dir_all(cache_root);
        let _ = image::save_buffer(
            path,
            &rgba,
            PREVIEW_CELL,
            PREVIEW_CELL,
            image::ColorType::Rgba8,
        );
    }
    Ok(Some(PreparedPreview {
        rgba,
        from_disk_cache: false,
    }))
}

fn load_cell(path: &Path) -> Option<Vec<u8>> {
    let image = image::open(path).ok()?;
    Some(fit_to_cell(image))
}

fn decode_texture(path: &Path) -> Option<Vec<u8>> {
    Some(fit_to_cell(image::open(path).ok()?))
}

fn fit_to_cell(image: DynamicImage) -> Vec<u8> {
    let fitted = image.thumbnail(PREVIEW_CELL, PREVIEW_CELL).to_rgba8();
    let mut cell = vec![0_u8; (PREVIEW_CELL * PREVIEW_CELL * 4) as usize];
    let ox = (PREVIEW_CELL - fitted.width()) / 2;
    let oy = (PREVIEW_CELL - fitted.height()) / 2;
    for (x, y, pixel) in fitted.enumerate_pixels() {
        let offset = (((oy + y) * PREVIEW_CELL + ox + x) * 4) as usize;
        cell[offset..offset + 4].copy_from_slice(&pixel.0);
    }
    cell
}

fn render_mesh(path: &Path) -> Option<Vec<u8>> {
    let scene = load_gltf(path).ok()?;
    let triangles = scene_triangles(&scene);
    if triangles.is_empty() {
        return None;
    }
    let mut min = Vec3::splat(f32::INFINITY);
    let mut max = Vec3::splat(f32::NEG_INFINITY);
    for triangle in &triangles {
        for vertex in triangle {
            min = min.min(vertex.position);
            max = max.max(vertex.position);
        }
    }
    let centre = (min + max) * 0.5;
    let extent = (max - min).max_element().max(0.0001);
    let view = Mat4::from_rotation_x(-20_f32.to_radians())
        * Mat4::from_rotation_y(35_f32.to_radians())
        * Mat4::from_translation(-centre);
    let scale = MESH_RENDER_SIZE as f32 * 0.74 / extent;
    let mut color = vec![0_u8; (MESH_RENDER_SIZE * MESH_RENDER_SIZE * 4) as usize];
    let mut depth = vec![f32::INFINITY; (MESH_RENDER_SIZE * MESH_RENDER_SIZE) as usize];
    let light = Vec3::new(-0.35, 0.8, 0.55).normalize();
    for triangle in triangles {
        let mut screen = [(0_f32, 0_f32, 0_f32); 3];
        for (index, vertex) in triangle.iter().enumerate() {
            let p = view.transform_point3(vertex.position);
            screen[index] = (
                p.x * scale + MESH_RENDER_SIZE as f32 * 0.5,
                -p.y * scale + MESH_RENDER_SIZE as f32 * 0.5,
                p.z,
            );
        }
        let normal = view.transform_vector3(
            (triangle[1].position - triangle[0].position)
                .cross(triangle[2].position - triangle[0].position)
                .normalize_or_zero(),
        );
        let shade = (0.24 + normal.dot(light).max(0.0) * 0.76).clamp(0.0, 1.0);
        raster_triangle(screen, shade, &mut color, &mut depth);
    }
    crop_and_downsample(&color, MESH_RENDER_SIZE, MESH_RENDER_SIZE)
}

#[derive(Clone, Copy)]
struct PreviewVertex {
    position: Vec3,
}

fn scene_triangles(scene: &LoadedScene) -> Vec<[PreviewVertex; 3]> {
    let mut triangles = Vec::new();
    if scene.nodes.is_empty() {
        for mesh in &scene.meshes {
            append_mesh(mesh, Mat4::IDENTITY, &mut triangles);
        }
    } else {
        for node in &scene.nodes {
            if let Some(mesh) = node.mesh_index.and_then(|index| scene.meshes.get(index)) {
                append_mesh(mesh, node.transform, &mut triangles);
            }
        }
    }
    triangles
}

fn append_mesh(mesh: &LoadedMesh, transform: Mat4, out: &mut Vec<[PreviewVertex; 3]>) {
    for indices in mesh.indices.chunks_exact(3) {
        let Some(vertices) = indices
            .iter()
            .map(|index| mesh.vertices.get(*index as usize))
            .collect::<Option<Vec<&Vertex>>>()
        else {
            continue;
        };
        out.push(std::array::from_fn(|i| PreviewVertex {
            position: (transform * Vec4::from((Vec3::from(vertices[i].position), 1.0))).truncate(),
        }));
    }
}

fn raster_triangle(p: [(f32, f32, f32); 3], shade: f32, color: &mut [u8], depth: &mut [f32]) {
    let min_x = p
        .iter()
        .map(|v| v.0)
        .fold(f32::INFINITY, f32::min)
        .floor()
        .max(0.0) as u32;
    let max_x = p
        .iter()
        .map(|v| v.0)
        .fold(f32::NEG_INFINITY, f32::max)
        .ceil()
        .min((MESH_RENDER_SIZE - 1) as f32) as u32;
    let min_y = p
        .iter()
        .map(|v| v.1)
        .fold(f32::INFINITY, f32::min)
        .floor()
        .max(0.0) as u32;
    let max_y = p
        .iter()
        .map(|v| v.1)
        .fold(f32::NEG_INFINITY, f32::max)
        .ceil()
        .min((MESH_RENDER_SIZE - 1) as f32) as u32;
    let area = edge(p[0], p[1], (p[2].0, p[2].1));
    if area.abs() < 0.0001 || min_x > max_x || min_y > max_y {
        return;
    }
    for y in min_y..=max_y {
        for x in min_x..=max_x {
            let sample = (x as f32 + 0.5, y as f32 + 0.5);
            let w0 = edge(p[1], p[2], sample) / area;
            let w1 = edge(p[2], p[0], sample) / area;
            let w2 = 1.0 - w0 - w1;
            if w0 < 0.0 || w1 < 0.0 || w2 < 0.0 {
                continue;
            }
            let z = w0 * p[0].2 + w1 * p[1].2 + w2 * p[2].2;
            let index = (y * MESH_RENDER_SIZE + x) as usize;
            if z >= depth[index] {
                continue;
            }
            depth[index] = z;
            let offset = index * 4;
            color[offset] = (72.0 + 94.0 * shade) as u8;
            color[offset + 1] = (82.0 + 105.0 * shade) as u8;
            color[offset + 2] = (96.0 + 120.0 * shade) as u8;
            color[offset + 3] = 255;
        }
    }
}

fn edge(a: (f32, f32, f32), b: (f32, f32, f32), p: (f32, f32)) -> f32 {
    (p.0 - a.0) * (b.1 - a.1) - (p.1 - a.1) * (b.0 - a.0)
}

fn crop_and_downsample(rgba: &[u8], width: u32, height: u32) -> Option<Vec<u8>> {
    let mut min_x = width;
    let mut min_y = height;
    let mut max_x = 0;
    let mut max_y = 0;
    let mut found = false;
    for y in 0..height {
        for x in 0..width {
            if rgba[((y * width + x) * 4 + 3) as usize] != 0 {
                found = true;
                min_x = min_x.min(x);
                min_y = min_y.min(y);
                max_x = max_x.max(x);
                max_y = max_y.max(y);
            }
        }
    }
    if !found {
        return None;
    }
    const BORDER: u32 = 3;
    min_x = min_x.saturating_sub(BORDER);
    min_y = min_y.saturating_sub(BORDER);
    max_x = (max_x + BORDER).min(width - 1);
    max_y = (max_y + BORDER).min(height - 1);
    let source = ImageBuffer::<Rgba<u8>, _>::from_raw(width, height, rgba.to_vec())?;
    let cropped =
        image::imageops::crop_imm(&source, min_x, min_y, max_x - min_x + 1, max_y - min_y + 1)
            .to_image();
    let mut side = cropped.width().max(cropped.height());
    side = side.max(1);
    let mut square = ImageBuffer::<Rgba<u8>, Vec<u8>>::new(side, side);
    image::imageops::overlay(
        &mut square,
        &cropped,
        i64::from((side - cropped.width()) / 2),
        i64::from((side - cropped.height()) / 2),
    );
    Some(
        image::imageops::resize(&square, PREVIEW_CELL, PREVIEW_CELL, FilterType::Triangle)
            .into_raw(),
    )
}

fn script_card() -> Vec<u8> {
    let mut image = vec![0_u8; (PREVIEW_CELL * PREVIEW_CELL * 4) as usize];
    for y in 8..56 {
        for x in 10..54 {
            let offset = ((y * PREVIEW_CELL + x) * 4) as usize;
            image[offset..offset + 4].copy_from_slice(&[36, 40, 50, 255]);
        }
    }
    for (row, width) in [30, 36, 24, 33, 20].into_iter().enumerate() {
        let y = 17 + row as u32 * 7;
        for x in 16..16 + width {
            let offset = ((y * PREVIEW_CELL + x) * 4) as usize;
            image[offset..offset + 4].copy_from_slice(&[198, 137, 72, 255]);
        }
    }
    image
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::database::{AssetId, AssetMetadata};
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicU64, Ordering};

    static NEXT_FIXTURE: AtomicU64 = AtomicU64::new(1);

    fn record(path: PathBuf, kind: AssetKind, hash: &str) -> AssetRecord {
        AssetRecord {
            id: AssetId::from_relative_path("preview.png"),
            relative_path: "preview.png".into(),
            absolute_path: path,
            name: "preview.png".into(),
            parent: String::new(),
            kind,
            metadata: AssetMetadata {
                content_hash: hash.into(),
                ..Default::default()
            },
        }
    }

    #[test]
    fn auto_crop_makes_a_thin_subject_fill_the_cell() {
        let mut image = vec![0_u8; (96 * 96 * 4) as usize];
        for y in 10..86 {
            let offset = ((y * 96 + 48) * 4) as usize;
            image[offset..offset + 4].copy_from_slice(&[255, 255, 255, 255]);
        }
        let result = crop_and_downsample(&image, 96, 96).unwrap();
        let occupied = result.chunks_exact(4).filter(|pixel| pixel[3] != 0).count();
        assert!(
            occupied > 100,
            "thin mesh stayed a speck: {occupied} pixels"
        );
    }

    #[test]
    fn script_fallback_is_never_empty() {
        assert!(script_card().chunks_exact(4).any(|pixel| pixel[3] != 0));
    }

    #[test]
    fn registry_decline_falls_through_and_exposes_frequency() {
        let mut registry = PreviewGeneratorRegistry::default();
        registry.register(PreviewGenerator {
            name: "declines",
            kind: AssetKind::Script,
            frequency: PreviewFrequency::Realtime,
            generate: |_| None,
        });
        registry.register(PreviewGenerator {
            name: "answer",
            kind: AssetKind::Script,
            frequency: PreviewFrequency::Once,
            generate: |_| Some(script_card()),
        });
        let asset = record(PathBuf::from("missing.luau"), AssetKind::Script, "script");
        assert!(registry.generate(&asset).is_some());
        assert_eq!(registry.entries()[0].frequency, PreviewFrequency::Realtime);
    }

    #[test]
    fn second_run_loads_the_warm_disk_cache() {
        let root = std::env::temp_dir().join(format!(
            "somnium_preview_{}_{}",
            std::process::id(),
            NEXT_FIXTURE.fetch_add(1, Ordering::Relaxed)
        ));
        let source = root.join("source.png");
        let cache = root.join(".somnium/thumbnails");
        fs::create_dir_all(&root).unwrap();
        image::save_buffer(&source, &[220_u8; 4 * 2 * 2], 2, 2, image::ColorType::Rgba8).unwrap();
        let asset = record(source, AssetKind::Texture, "stable-hash");
        let first = prepare_preview(&asset, &cache).unwrap().unwrap();
        assert!(!first.from_disk_cache);
        let second = prepare_preview(&asset, &cache).unwrap().unwrap();
        assert!(second.from_disk_cache);
        assert_eq!(first.rgba, second.rgba);
        let _ = fs::remove_dir_all(root);
    }
}
