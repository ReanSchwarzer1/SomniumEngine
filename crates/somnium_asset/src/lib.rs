//! glTF 2.0 asset loader for the Somnium Engine.
//!
//! Loads `.gltf` / `.glb` files and converts their content into
//! Somnium-native types (`LoadedScene`) that the renderer can upload
//! without ever seeing `gltf::` crate types directly.

use std::path::Path;
use std::collections::HashMap;

use bytemuck::{Pod, Zeroable};
use glam::{Mat4, Vec3};
use tracing::{info, warn};

// ── Vertex (32 bytes) ──────────────────────────────────────────────────────

/// Vertex layout used by the Visibility Buffer pipeline.
/// Must match the WGSL `struct Vertex` in visibility.wgsl / shading.wgsl.
#[repr(C)]
#[derive(Copy, Clone, Debug, Pod, Zeroable)]
pub struct Vertex {
    pub position: [f32; 3],  // 12 bytes
    pub normal:   [f32; 3],  // 12 bytes
    pub uv:       [f32; 2],  //  8 bytes
}                             // = 32 bytes

// ── Loaded types (Somnium-native, no gltf:: types) ────────────────────────

/// Raw RGBA8 image decoded from a glTF texture.
pub struct LoadedTexture {
    pub data:   Vec<u8>, // row-major RGBA8 pixels
    pub width:  u32,
    pub height: u32,
}

/// PBR metallic-roughness material. Texture indices reference `LoadedScene.textures`.
pub struct LoadedMaterial {
    pub base_color:            [f32; 4],
    pub roughness:             f32,
    pub metallic:              f32,
    pub albedo_map:            Option<usize>,
    pub normal_map:            Option<usize>,
    pub metallic_roughness_map: Option<usize>,
}

/// A single mesh primitive (position + normal + UV geometry + triangle indices).
pub struct LoadedMesh {
    pub vertices: Vec<Vertex>,
    pub indices:  Vec<u32>,
}

/// A scene node flattened to world-space. Mesh/material indices reference
/// the parallel `meshes` and `materials` vecs in `LoadedScene`.
pub struct SceneNode {
    pub name:           String,
    pub mesh_index:     Option<usize>,
    pub material_index: Option<usize>,
    pub transform:      Mat4, // world-space (parent chain already multiplied)
}

/// The result of `load_gltf`. Everything the renderer needs to upload a scene.
#[derive(Default)]
pub struct LoadedScene {
    pub meshes:    Vec<LoadedMesh>,
    pub materials: Vec<LoadedMaterial>,
    pub textures:  Vec<LoadedTexture>,
    pub nodes:     Vec<SceneNode>,
}

// ── Public API ─────────────────────────────────────────────────────────────

/// Load a glTF 2.0 / GLB file and return a `LoadedScene`.
///
/// No `gltf::` types escape this function — callers only see Somnium types.
pub fn load_gltf(path: impl AsRef<Path>) -> Result<LoadedScene, String> {
    let path = path.as_ref();
    info!("Loading glTF: {:?}", path);

    let (document, buffers, images) = gltf::import(path)
        .map_err(|e| format!("glTF import failed: {e}"))?;

    let mut scene = LoadedScene::default();

    // 1. Textures → RGBA8 -----------------------------------------------
    for img in &images {
        scene.textures.push(decode_to_rgba8(img));
    }

    // 2. Materials -------------------------------------------------------
    for mat in document.materials() {
        let pbr = mat.pbr_metallic_roughness();
        scene.materials.push(LoadedMaterial {
            base_color:            pbr.base_color_factor(),
            roughness:             pbr.roughness_factor(),
            metallic:              pbr.metallic_factor(),
            albedo_map:            pbr.base_color_texture().map(|t| t.texture().source().index()),
            normal_map:            mat.normal_texture().map(|t| t.texture().source().index()),
            metallic_roughness_map: pbr.metallic_roughness_texture()
                                       .map(|t| t.texture().source().index()),
        });
    }
    // Ensure there is always at least a default material at index 0.
    if scene.materials.is_empty() {
        scene.materials.push(LoadedMaterial {
            base_color:            [1.0, 1.0, 1.0, 1.0],
            roughness:             0.5,
            metallic:              0.0,
            albedo_map:            None,
            normal_map:            None,
            metallic_roughness_map: None,
        });
    }

    // 3. Meshes (one LoadedMesh per glTF primitive) ----------------------
    // Map (mesh_index, prim_index) → index into scene.meshes
    let mut prim_to_loaded: HashMap<(usize, usize), usize> = HashMap::new();

    for mesh in document.meshes() {
        let prim_count = mesh.primitives().count();
        for (prim_idx, primitive) in mesh.primitives().enumerate() {
            let reader = primitive.reader(|buf| Some(&buffers[buf.index()]));

            let positions: Vec<[f32; 3]> = reader
                .read_positions()
                .ok_or_else(|| {
                    format!(
                        "Mesh '{}' primitive {prim_idx}: no POSITION attribute",
                        mesh.name().unwrap_or("?")
                    )
                })?
                .collect();

            let normals: Option<Vec<[f32; 3]>> =
                reader.read_normals().map(|n| n.collect());
            let uvs: Option<Vec<[f32; 2]>> =
                reader.read_tex_coords(0).map(|u| u.into_f32().collect());

            let mut vertices: Vec<Vertex> = positions
                .iter()
                .enumerate()
                .map(|(i, &pos)| Vertex {
                    position: pos,
                    normal: normals.as_ref().and_then(|n| n.get(i).copied())
                        .unwrap_or([0.0, 1.0, 0.0]),
                    uv: uvs.as_ref().and_then(|u| u.get(i).copied())
                        .unwrap_or([0.0, 0.0]),
                })
                .collect();

            let indices: Vec<u32> = reader
                .read_indices()
                .ok_or_else(|| {
                    format!(
                        "Mesh '{}' primitive {prim_idx}: no index buffer",
                        mesh.name().unwrap_or("?")
                    )
                })?
                .into_u32()
                .collect();

            if normals.is_none() {
                generate_flat_normals(&mut vertices, &indices);
            }

            let loaded_idx = scene.meshes.len();
            prim_to_loaded.insert((mesh.index(), prim_idx), loaded_idx);

            let _ = prim_count; // used above for naming
            scene.meshes.push(LoadedMesh { vertices, indices });
        }
    }

    // 4. Scene graph → flat world-space nodes ----------------------------
    let gltf_scene = document.default_scene().or_else(|| document.scenes().next());
    match gltf_scene {
        Some(s) => {
            for root in s.nodes() {
                collect_nodes(&root, Mat4::IDENTITY, &prim_to_loaded, &mut scene.nodes);
            }
        }
        None => warn!("glTF file has no scenes; no nodes spawned"),
    }

    info!(
        "glTF loaded: {} meshes, {} materials, {} textures, {} nodes",
        scene.meshes.len(),
        scene.materials.len(),
        scene.textures.len(),
        scene.nodes.len(),
    );
    Ok(scene)
}

// ── Internal helpers ───────────────────────────────────────────────────────

fn collect_nodes(
    node:           &gltf::Node,
    parent_xform:   Mat4,
    prim_to_loaded: &HashMap<(usize, usize), usize>,
    out:            &mut Vec<SceneNode>,
) {
    let local = Mat4::from_cols_array_2d(&node.transform().matrix());
    let world = parent_xform * local;
    let node_name = node.name().unwrap_or("Node").to_string();

    if let Some(mesh) = node.mesh() {
        let prim_count = mesh.primitives().count();
        for (prim_idx, primitive) in mesh.primitives().enumerate() {
            let mesh_index     = prim_to_loaded.get(&(mesh.index(), prim_idx)).copied();
            let material_index = primitive.material().index();
            let name = if prim_count == 1 {
                node_name.clone()
            } else {
                format!("{node_name}_{prim_idx}")
            };
            out.push(SceneNode { name, mesh_index, material_index, transform: world });
        }
    }

    for child in node.children() {
        collect_nodes(&child, world, prim_to_loaded, out);
    }
}

/// Convert any gltf image format to tightly-packed RGBA8.
fn decode_to_rgba8(data: &gltf::image::Data) -> LoadedTexture {
    use gltf::image::Format;
    let rgba: Vec<u8> = match data.format {
        Format::R8G8B8A8 => data.pixels.clone(),
        Format::R8G8B8 => data.pixels.chunks_exact(3)
            .flat_map(|rgb| [rgb[0], rgb[1], rgb[2], 255])
            .collect(),
        Format::R8 => data.pixels.iter()
            .flat_map(|&r| [r, r, r, 255])
            .collect(),
        Format::R8G8 => data.pixels.chunks_exact(2)
            .flat_map(|rg| [rg[0], rg[1], 0, 255])
            .collect(),
        _ => {
            warn!("Unsupported texture format {:?}; using magenta placeholder", data.format);
            [255u8, 0, 255, 255].iter()
                .cycle()
                .take((data.width * data.height * 4) as usize)
                .copied()
                .collect()
        }
    };
    LoadedTexture { data: rgba, width: data.width, height: data.height }
}

// ── Phase 11.5D-2: Procedural Mesh Generation ─────────────────────────────

/// Generate a unit cube (1×1×1) centred at the origin with proper normals and UVs.
pub fn generate_cube(size: f32) -> (Vec<Vertex>, Vec<u32>) {
    let h = size * 0.5;
    #[rustfmt::skip]
    let vertices = vec![
        // Front (+Z)
        Vertex { position: [-h, -h,  h], normal: [0.0, 0.0, 1.0], uv: [0.0, 1.0] },
        Vertex { position: [ h, -h,  h], normal: [0.0, 0.0, 1.0], uv: [1.0, 1.0] },
        Vertex { position: [ h,  h,  h], normal: [0.0, 0.0, 1.0], uv: [1.0, 0.0] },
        Vertex { position: [-h,  h,  h], normal: [0.0, 0.0, 1.0], uv: [0.0, 0.0] },
        // Back (−Z)
        Vertex { position: [-h, -h, -h], normal: [0.0, 0.0,-1.0], uv: [1.0, 1.0] },
        Vertex { position: [-h,  h, -h], normal: [0.0, 0.0,-1.0], uv: [1.0, 0.0] },
        Vertex { position: [ h,  h, -h], normal: [0.0, 0.0,-1.0], uv: [0.0, 0.0] },
        Vertex { position: [ h, -h, -h], normal: [0.0, 0.0,-1.0], uv: [0.0, 1.0] },
        // Top (+Y)
        Vertex { position: [-h,  h, -h], normal: [0.0, 1.0, 0.0], uv: [0.0, 1.0] },
        Vertex { position: [-h,  h,  h], normal: [0.0, 1.0, 0.0], uv: [1.0, 1.0] },
        Vertex { position: [ h,  h,  h], normal: [0.0, 1.0, 0.0], uv: [1.0, 0.0] },
        Vertex { position: [ h,  h, -h], normal: [0.0, 1.0, 0.0], uv: [0.0, 0.0] },
        // Bottom (−Y)
        Vertex { position: [-h, -h, -h], normal: [0.0,-1.0, 0.0], uv: [1.0, 1.0] },
        Vertex { position: [ h, -h, -h], normal: [0.0,-1.0, 0.0], uv: [1.0, 0.0] },
        Vertex { position: [ h, -h,  h], normal: [0.0,-1.0, 0.0], uv: [0.0, 0.0] },
        Vertex { position: [-h, -h,  h], normal: [0.0,-1.0, 0.0], uv: [0.0, 1.0] },
        // Right (+X)
        Vertex { position: [ h, -h, -h], normal: [1.0, 0.0, 0.0], uv: [0.0, 1.0] },
        Vertex { position: [ h,  h, -h], normal: [1.0, 0.0, 0.0], uv: [1.0, 1.0] },
        Vertex { position: [ h,  h,  h], normal: [1.0, 0.0, 0.0], uv: [1.0, 0.0] },
        Vertex { position: [ h, -h,  h], normal: [1.0, 0.0, 0.0], uv: [0.0, 0.0] },
        // Left (−X)
        Vertex { position: [-h, -h, -h], normal: [-1.0, 0.0, 0.0], uv: [1.0, 1.0] },
        Vertex { position: [-h, -h,  h], normal: [-1.0, 0.0, 0.0], uv: [1.0, 0.0] },
        Vertex { position: [-h,  h,  h], normal: [-1.0, 0.0, 0.0], uv: [0.0, 0.0] },
        Vertex { position: [-h,  h, -h], normal: [-1.0, 0.0, 0.0], uv: [0.0, 1.0] },
    ];
    let indices = vec![
         0,  1,  2,  2,  3,  0,
         4,  5,  6,  6,  7,  4,
         8,  9, 10, 10, 11,  8,
        12, 13, 14, 14, 15, 12,
        16, 17, 18, 18, 19, 16,
        20, 21, 22, 22, 23, 20,
    ];
    (vertices, indices)
}

/// Generate a UV sphere with `segments` longitudinal slices and `rings` latitudinal bands.
pub fn generate_sphere(radius: f32, segments: u32, rings: u32) -> (Vec<Vertex>, Vec<u32>) {
    let segments = segments.max(3);
    let rings = rings.max(2);
    let mut vertices = Vec::new();
    let mut indices = Vec::new();

    for ring in 0..=rings {
        let phi = std::f32::consts::PI * ring as f32 / rings as f32;
        let sin_phi = phi.sin();
        let cos_phi = phi.cos();
        for seg in 0..=segments {
            let theta = 2.0 * std::f32::consts::PI * seg as f32 / segments as f32;
            let sin_t = theta.sin();
            let cos_t = theta.cos();
            let nx = sin_phi * cos_t;
            let ny = cos_phi;
            let nz = sin_phi * sin_t;
            vertices.push(Vertex {
                position: [nx * radius, ny * radius, nz * radius],
                normal:   [nx, ny, nz],
                uv:       [seg as f32 / segments as f32, ring as f32 / rings as f32],
            });
        }
    }

    for ring in 0..rings {
        for seg in 0..segments {
            let cur = ring * (segments + 1) + seg;
            let nxt = cur + segments + 1;
            indices.extend_from_slice(&[cur, nxt, cur + 1, cur + 1, nxt, nxt + 1]);
        }
    }
    (vertices, indices)
}

/// Generate a flat XZ plane centred at the origin with optional subdivisions.
pub fn generate_plane(size: f32, subdivisions: u32) -> (Vec<Vertex>, Vec<u32>) {
    let divs = subdivisions.max(1);
    let h = size * 0.5;
    let step = size / divs as f32;
    let mut vertices = Vec::new();
    let mut indices = Vec::new();

    for iz in 0..=divs {
        for ix in 0..=divs {
            let x = -h + ix as f32 * step;
            let z = -h + iz as f32 * step;
            vertices.push(Vertex {
                position: [x, 0.0, z],
                normal:   [0.0, 1.0, 0.0],
                uv:       [ix as f32 / divs as f32, iz as f32 / divs as f32],
            });
        }
    }

    let row = divs + 1;
    for iz in 0..divs {
        for ix in 0..divs {
            let base = iz * row + ix;
            indices.extend_from_slice(&[base, base + row, base + 1, base + 1, base + row, base + row + 1]);
        }
    }
    (vertices, indices)
}

/// Generate a capped cylinder with `segments` slices.
pub fn generate_cylinder(radius: f32, height: f32, segments: u32) -> (Vec<Vertex>, Vec<u32>) {
    let segments = segments.max(3);
    let mut vertices = Vec::new();
    let mut indices: Vec<u32> = Vec::new();
    let h = height * 0.5;

    // Side wall
    let side_base = vertices.len() as u32;
    for seg in 0..=segments {
        let theta = 2.0 * std::f32::consts::PI * seg as f32 / segments as f32;
        let c = theta.cos();
        let s = theta.sin();
        let u = seg as f32 / segments as f32;
        vertices.push(Vertex { position: [c * radius, -h, s * radius], normal: [c, 0.0, s], uv: [u, 1.0] });
        vertices.push(Vertex { position: [c * radius,  h, s * radius], normal: [c, 0.0, s], uv: [u, 0.0] });
    }
    for seg in 0..segments {
        let b = side_base + seg * 2;
        indices.extend_from_slice(&[b, b + 1, b + 2, b + 2, b + 1, b + 3]);
    }

    // Top cap
    let top_centre = vertices.len() as u32;
    vertices.push(Vertex { position: [0.0, h, 0.0], normal: [0.0, 1.0, 0.0], uv: [0.5, 0.5] });
    let top_ring_start = vertices.len() as u32;
    for seg in 0..segments {
        let theta = 2.0 * std::f32::consts::PI * seg as f32 / segments as f32;
        let c = theta.cos(); let s = theta.sin();
        vertices.push(Vertex { position: [c * radius, h, s * radius], normal: [0.0, 1.0, 0.0], uv: [c * 0.5 + 0.5, s * 0.5 + 0.5] });
    }
    for seg in 0..segments {
        let a = top_ring_start + seg;
        let b = top_ring_start + (seg + 1) % segments;
        indices.extend_from_slice(&[top_centre, a, b]);
    }

    // Bottom cap
    let bot_centre = vertices.len() as u32;
    vertices.push(Vertex { position: [0.0, -h, 0.0], normal: [0.0,-1.0, 0.0], uv: [0.5, 0.5] });
    let bot_ring_start = vertices.len() as u32;
    for seg in 0..segments {
        let theta = 2.0 * std::f32::consts::PI * seg as f32 / segments as f32;
        let c = theta.cos(); let s = theta.sin();
        vertices.push(Vertex { position: [c * radius, -h, s * radius], normal: [0.0,-1.0, 0.0], uv: [c * 0.5 + 0.5, s * 0.5 + 0.5] });
    }
    for seg in 0..segments {
        let a = bot_ring_start + seg;
        let b = bot_ring_start + (seg + 1) % segments;
        indices.extend_from_slice(&[bot_centre, b, a]);
    }

    (vertices, indices)
}

/// Generate per-face (flat) normals for a mesh that has no normal attribute.
fn generate_flat_normals(vertices: &mut [Vertex], indices: &[u32]) {
    for tri in indices.chunks_exact(3) {
        let i0 = tri[0] as usize;
        let i1 = tri[1] as usize;
        let i2 = tri[2] as usize;
        let p0 = Vec3::from(vertices[i0].position);
        let p1 = Vec3::from(vertices[i1].position);
        let p2 = Vec3::from(vertices[i2].position);
        let n = (p1 - p0).cross(p2 - p0).normalize_or_zero().to_array();
        vertices[i0].normal = n;
        vertices[i1].normal = n;
        vertices[i2].normal = n;
    }
}
