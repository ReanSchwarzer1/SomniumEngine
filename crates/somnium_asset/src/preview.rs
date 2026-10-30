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
/// Rendered at three times the cell and filtered down: the edges come out smooth.
const MESH_RENDER_SIZE: u32 = 192;

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
    render_sphere(material, None)
}

/// [`prepare_preview`] for a material whose base colour is a texture, already found by the host (which knows asset
/// ids; the worker does not). The sphere wears the texture, so a drawer of materials no longer looks like a drawer of
/// identical white balls (Phase ALYX-E). Cached apart from the untextured cell.
pub fn prepare_material_preview(
    record: &AssetRecord,
    cache_root: &Path,
    albedo: &Path,
) -> Result<Option<PreparedPreview>, String> {
    let hash = &record.metadata.content_hash;
    let cache_path = (!hash.is_empty()).then(|| cache_root.join(format!("{hash}-albedo.png")));
    if let Some(path) = cache_path.as_deref() {
        if let Some(rgba) = load_cell(path) {
            return Ok(Some(PreparedPreview { rgba, from_disk_cache: true }));
        }
    }
    let material = load_material(&record.absolute_path)?;
    let rgba = match material.preview_png().and_then(|png| image::load_from_memory_with_format(&png, image::ImageFormat::Png).ok()) {
        Some(image) => fit_to_cell(image),
        None => {
            // A 128-texel copy is plenty for a 64-pixel ball and keeps the worker quick on 4K maps.
            let texture = image::open(albedo).ok().map(|image| image.thumbnail(128, 128).to_rgba8());
            render_sphere(&material, texture.as_ref())
        }
    };
    if let Some(path) = cache_path.as_deref() {
        let _ = fs::create_dir_all(cache_root);
        let _ = image::save_buffer(path, &rgba, PREVIEW_CELL, PREVIEW_CELL, image::ColorType::Rgba8);
    }
    Ok(Some(PreparedPreview { rgba, from_disk_cache: false }))
}

/// The studio sphere, optionally wearing its base-colour texture (sRGB texels, taken to linear and multiplied by the
/// base colour), mapped by longitude and latitude and repeated twice around.
fn render_sphere(material: &MaterialAsset, albedo: Option<&image::RgbaImage>) -> Vec<u8> {
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
            let base = match albedo {
                Some(texture) => {
                    let u = (0.5 + normal.x.atan2(normal.z) / std::f32::consts::TAU) * 2.0;
                    let v = normal.y.clamp(-1.0, 1.0).acos() / std::f32::consts::PI;
                    let tx = ((u.rem_euclid(1.0) * texture.width() as f32) as u32).min(texture.width() - 1);
                    let ty = ((v * texture.height() as f32) as u32).min(texture.height() - 1);
                    let texel = texture.get_pixel(tx, ty).0;
                    let linear = Vec3::new(texel[0] as f32, texel[1] as f32, texel[2] as f32) / 255.0;
                    base * linear.powf(2.2)
                }
                None => base,
            };
            let diffuse = normal.dot(light).max(0.0);
            let half = (light + view).normalize();
            let gloss = (1.0 - material.roughness).clamp(0.0, 1.0);
            let specular = normal.dot(half).max(0.0).powf(2.0 + gloss * 126.0);
            let f0 = Vec3::splat(0.04).lerp(base, material.metallic.clamp(0.0, 1.0));
            let diffuse_color = base * (1.0 - material.metallic.clamp(0.0, 1.0));
            // What the ball reflects: a soft sky above a dark ground, blurred toward its middle grey as it roughens,
            // with Schlick's rise toward the rim. Without it a metal, which has no diffuse light, was a black ball.
            let reflected = (normal * 2.0 * normal.dot(view) - view).normalize();
            let sky = 0.06 + 0.5 * (reflected.y * 0.5 + 0.5).powf(1.5);
            let environment = sky + (0.22 - sky) * material.roughness.clamp(0.0, 1.0);
            let fresnel = f0 + (Vec3::ONE - f0) * (1.0 - normal.dot(view).max(0.0)).powi(5) * gloss;
            let color = diffuse_color * (0.08 + diffuse * 0.72)
                + f0 * specular * (0.35 + gloss * 1.8)
                + fresnel * environment
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
    // Models were grey silhouettes until the studio renders: their old cells must not be read back as current.
    let version = if record.kind == AssetKind::Mesh { "-studio" } else { "" };
    let cache_path = (!hash.is_empty()).then(|| cache_root.join(format!("{hash}{version}.png")));
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

/// Any image file on disk, fitted into one preview cell (`PREVIEW_CELL` square, RGBA8).
pub fn decode_texture(path: &Path) -> Option<Vec<u8>> {
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

/// A model's tile: the model in its own materials under a studio rig (Phase ALYX-J). It used to be a grey silhouette.
///
/// A three-quarter view from slightly above. Key light warm from the upper left, a cool fill from the right, a rim
/// from behind that lifts the edges off the dark tile, and a little sky above the ground. Base colour comes from
/// the material's factor and its albedo texture; masked cut-outs (leaves, fences) stay cut out.
fn render_mesh(path: &Path) -> Option<Vec<u8>> {
    let scene = load_gltf(path).ok()?;
    let triangles = scene_triangles(&scene);
    if triangles.is_empty() {
        return None;
    }
    let mut min = Vec3::splat(f32::INFINITY);
    let mut max = Vec3::splat(f32::NEG_INFINITY);
    for triangle in &triangles {
        for vertex in &triangle.vertices {
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
    let size = MESH_RENDER_SIZE as usize;
    let mut color = vec![0_u8; size * size * 4];
    let mut depth = vec![f32::INFINITY; size * size];
    for triangle in &triangles {
        let mut screen = [(0_f32, 0_f32, 0_f32); 3];
        let mut normals = [Vec3::ZERO; 3];
        for (index, vertex) in triangle.vertices.iter().enumerate() {
            let p = view.transform_point3(vertex.position);
            screen[index] = (
                p.x * scale + MESH_RENDER_SIZE as f32 * 0.5,
                -p.y * scale + MESH_RENDER_SIZE as f32 * 0.5,
                p.z,
            );
            normals[index] = view.transform_vector3(vertex.normal).normalize_or_zero();
        }
        let face = view
            .transform_vector3(
                (triangle.vertices[1].position - triangle.vertices[0].position)
                    .cross(triangle.vertices[2].position - triangle.vertices[0].position),
            )
            .normalize_or_zero();
        let material = triangle.material.and_then(|index| scene.materials.get(index));
        let texture = material
            .and_then(|m| m.albedo_map)
            .and_then(|index| scene.textures.get(index))
            .filter(|t| t.width > 1 && t.height > 1);
        let surface = Surface {
            base: material.map_or([0.72, 0.74, 0.78, 1.0], |m| m.base_color),
            roughness: material.map_or(0.6, |m| m.roughness),
            metallic: material.map_or(0.0, |m| m.metallic),
            cutoff: material
                .filter(|m| m.alpha_mode == crate::AlphaMode::Mask)
                .map(|m| m.alpha_cutoff),
            texture,
        };
        raster_triangle(screen, &triangle.vertices, normals, face, &surface, &mut color, &mut depth);
    }
    crop_and_downsample(&color, MESH_RENDER_SIZE, MESH_RENDER_SIZE)
}

#[derive(Clone, Copy)]
struct PreviewVertex {
    position: Vec3,
    normal: Vec3,
    uv: [f32; 2],
}

struct PreviewTriangle {
    vertices: [PreviewVertex; 3],
    material: Option<usize>,
}

struct Surface<'a> {
    base: [f32; 4],
    roughness: f32,
    metallic: f32,
    cutoff: Option<f32>,
    texture: Option<&'a crate::LoadedTexture>,
}

impl Surface<'_> {
    /// Linear base colour and coverage at `uv`.
    fn albedo(&self, uv: [f32; 2]) -> (Vec3, f32) {
        let base = Vec3::new(self.base[0], self.base[1], self.base[2]);
        let Some(texture) = self.texture else {
            return (base, self.base[3]);
        };
        let x = ((uv[0].rem_euclid(1.0) * texture.width as f32) as u32).min(texture.width - 1);
        let y = ((uv[1].rem_euclid(1.0) * texture.height as f32) as u32).min(texture.height - 1);
        let at = ((y * texture.width + x) * 4) as usize;
        let Some(texel) = texture.data.get(at..at + 4) else {
            return (base, self.base[3]);
        };
        let srgb = Vec3::new(f32::from(texel[0]), f32::from(texel[1]), f32::from(texel[2])) / 255.0;
        (base * srgb.powf(2.2), self.base[3] * f32::from(texel[3]) / 255.0)
    }
}

/// The studio rig in view space (the camera looks down -Z), and the colour a surface leaves with.
fn shade(albedo: Vec3, normal: Vec3, surface: &Surface) -> Vec3 {
    let key = Vec3::new(-0.45, 0.7, 0.55).normalize();
    let fill = Vec3::new(0.75, 0.15, 0.6).normalize();
    let eye = Vec3::Z;
    let diffuse_colour = albedo * (1.0 - surface.metallic);
    let lambert = |light: Vec3| normal.dot(light).max(0.0);
    let half = (key + eye).normalize();
    let shininess = 2.0 + (1.0 - surface.roughness.clamp(0.0, 1.0)).powi(2) * 90.0;
    let specular_colour = Vec3::splat(0.04).lerp(albedo, surface.metallic);
    let specular = specular_colour * normal.dot(half).max(0.0).powf(shininess) * (shininess + 2.0) / 8.0;
    let rim = (1.0 - normal.dot(eye).max(0.0)).powi(4) * 0.4;
    let sky = 0.10 + 0.06 * normal.y;
    let warm = Vec3::new(1.0, 0.95, 0.88);
    let cool = Vec3::new(0.74, 0.82, 1.0);
    diffuse_colour * (warm * lambert(key) * 1.25 + cool * lambert(fill) * 0.38 + Vec3::splat(sky))
        + specular * warm * lambert(key).min(1.0) * 0.9
        + cool * rim
}

/// The ACES filmic fit (Narkowicz), then sRGB: a lit tile, not a clipped one, and colours that stay saturated.
fn encode(colour: Vec3) -> [u8; 3] {
    let x = colour * 1.1;
    let mapped = (x * (x * 2.51 + Vec3::splat(0.03))) / (x * (x * 2.43 + Vec3::splat(0.59)) + Vec3::splat(0.14));
    let srgb = mapped.clamp(Vec3::ZERO, Vec3::ONE).powf(1.0 / 2.2);
    [(srgb.x * 255.0) as u8, (srgb.y * 255.0) as u8, (srgb.z * 255.0) as u8]
}

fn scene_triangles(scene: &LoadedScene) -> Vec<PreviewTriangle> {
    let mut triangles = Vec::new();
    if scene.nodes.is_empty() {
        for mesh in &scene.meshes {
            append_mesh(mesh, Mat4::IDENTITY, None, &mut triangles);
        }
    } else {
        for node in &scene.nodes {
            if let Some(mesh) = node.mesh_index.and_then(|index| scene.meshes.get(index)) {
                append_mesh(mesh, node.transform, node.material_index, &mut triangles);
            }
        }
    }
    triangles
}

fn append_mesh(mesh: &LoadedMesh, transform: Mat4, material: Option<usize>, out: &mut Vec<PreviewTriangle>) {
    let normal_matrix = transform.inverse().transpose();
    for indices in mesh.indices.chunks_exact(3) {
        let Some(vertices) = indices
            .iter()
            .map(|index| mesh.vertices.get(*index as usize))
            .collect::<Option<Vec<&Vertex>>>()
        else {
            continue;
        };
        out.push(PreviewTriangle {
            vertices: std::array::from_fn(|i| PreviewVertex {
                position: (transform * Vec4::from((Vec3::from(vertices[i].position), 1.0))).truncate(),
                normal: normal_matrix.transform_vector3(Vec3::from(vertices[i].normal)).normalize_or_zero(),
                uv: vertices[i].uv,
            }),
            material,
        });
    }
}

#[allow(clippy::too_many_arguments)]
fn raster_triangle(
    p: [(f32, f32, f32); 3],
    vertices: &[PreviewVertex; 3],
    normals: [Vec3; 3],
    face: Vec3,
    surface: &Surface,
    color: &mut [u8],
    depth: &mut [f32],
) {
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
            let uv = [
                w0 * vertices[0].uv[0] + w1 * vertices[1].uv[0] + w2 * vertices[2].uv[0],
                w0 * vertices[0].uv[1] + w1 * vertices[1].uv[1] + w2 * vertices[2].uv[1],
            ];
            let (albedo, coverage) = surface.albedo(uv);
            if surface.cutoff.is_some_and(|cutoff| coverage < cutoff) {
                continue;
            }
            depth[index] = z;
            // Smooth normals where the file has them; the face where it does not. Facing away, flip: a thin
            // sheet seen from behind is still lit.
            let mut normal = (normals[0] * w0 + normals[1] * w1 + normals[2] * w2).normalize_or_zero();
            if normal == Vec3::ZERO {
                normal = face;
            }
            if normal.z < 0.0 && face.z < 0.0 {
                normal = -normal;
            }
            let rgb = encode(shade(albedo, normal, surface));
            let offset = index * 4;
            color[offset..offset + 3].copy_from_slice(&rgb);
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

    /// Render real models' tiles for a look: `SOMNIUM_THUMB_SOURCES` (paths split by `;`) into `SOMNIUM_THUMB_OUT`.
    #[test]
    #[ignore = "writes review images; run by hand"]
    fn render_tiles_for_review() {
        let (Ok(sources), Ok(out)) = (std::env::var("SOMNIUM_THUMB_SOURCES"), std::env::var("SOMNIUM_THUMB_OUT")) else {
            return;
        };
        let out = std::path::PathBuf::from(out);
        std::fs::create_dir_all(&out).unwrap();
        for source in sources.split(';').filter(|s| !s.is_empty()) {
            let source = std::path::Path::new(source);
            let Some(rgba) = render_mesh(source) else { continue };
            let name = source.file_stem().unwrap().to_string_lossy();
            image::save_buffer(out.join(format!("{name}.png")), &rgba, PREVIEW_CELL, PREVIEW_CELL, image::ColorType::Rgba8)
                .unwrap();
        }
    }

    #[test]
    fn the_studio_rig_lights_the_key_side_and_keeps_the_material_colour() {
        let surface = Surface { base: [0.8, 0.1, 0.1, 1.0], roughness: 0.7, metallic: 0.0, cutoff: None, texture: None };
        let (albedo, _) = surface.albedo([0.0, 0.0]);
        let lit = encode(shade(albedo, Vec3::new(-0.45, 0.7, 0.55).normalize(), &surface));
        let away = encode(shade(albedo, Vec3::new(0.4, -0.7, 0.2).normalize(), &surface));
        assert!(lit[0] > away[0].saturating_add(40), "key side {lit:?} against the far side {away:?}");
        assert!(f32::from(lit[0]) > f32::from(lit[1]) * 1.5, "a red material reads red: {lit:?}");
    }

    #[test]
    fn a_textured_material_wears_its_texture() {
        let material = MaterialAsset::default();
        let red = image::RgbaImage::from_pixel(4, 4, Rgba([220, 20, 20, 255]));
        let plain = render_sphere(&material, None);
        let worn = render_sphere(&material, Some(&red));
        let centre = (((PREVIEW_CELL / 2) * PREVIEW_CELL + PREVIEW_CELL / 2) * 4) as usize;
        assert!(worn[centre] > worn[centre + 1] * 3, "the ball should read red: {:?}", &worn[centre..centre + 3]);
        assert_ne!(plain[centre..centre + 3], worn[centre..centre + 3]);
    }

    /// A metal has no diffuse light, so its ball shows what it reflects; it must not come out black.
    #[test]
    fn a_metal_ball_reflects_rather_than_going_black() {
        let metal = MaterialAsset { base_color: crate::material::LinearColor([0.56, 0.57, 0.58]), metallic: 1.0, roughness: 0.3, ..Default::default() };
        let cell = render_sphere(&metal, None);
        let lit: Vec<_> = cell.chunks_exact(4).filter(|p| p[3] != 0).collect();
        let mean = lit.iter().map(|p| u32::from(p[1])).sum::<u32>() / lit.len() as u32;
        assert!(mean > 70, "the metal ball reads as metal, not a hole: mean green {mean}");
        // Upper half (the sky) brighter than the lower (the ground).
        let row = |y: u32| u32::from(cell[((y * PREVIEW_CELL + PREVIEW_CELL / 2) * 4 + 1) as usize]);
        assert!(row(PREVIEW_CELL / 4) > row(PREVIEW_CELL * 3 / 4), "{} vs {}", row(PREVIEW_CELL / 4), row(PREVIEW_CELL * 3 / 4));
    }
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
