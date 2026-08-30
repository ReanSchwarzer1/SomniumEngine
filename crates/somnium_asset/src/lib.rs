//! glTF 2.0 asset loader for the Somnium Engine.
//!
//! Loads `.gltf` / `.glb` files and converts their content into
//! Somnium-native types (`LoadedScene`) that the renderer can upload
//! without ever seeing `gltf::` crate types directly.

pub mod cook;
pub mod database;
/// MORROWIND-M item 3: what references what, across a whole project.
pub mod depend;
pub mod material;
pub mod preview;
pub mod residency;
pub mod scene_file;
/// Logical terrain source-page addresses used by MORROWIND-AD virtual texturing.
pub mod virtual_texture;
/// MORROWIND-T offline HLOD and octahedral-impostor baking.
pub mod world_bake;

use std::collections::HashMap;
use std::path::Path;

use bytemuck::{Pod, Zeroable};
use glam::{Mat4, Vec3};
use tracing::{info, warn};

// ── Vertex (32 bytes) ──────────────────────────────────────────────────────

/// Vertex layout used by the Visibility Buffer pipeline.
/// Must match the WGSL `struct Vertex` in visibility.wgsl / shading.wgsl.
#[repr(C)]
#[derive(Copy, Clone, Debug, Pod, Zeroable)]
pub struct Vertex {
    pub position: [f32; 3], // 12 bytes
    pub normal: [f32; 3],   // 12 bytes
    pub uv: [f32; 2],       //  8 bytes
} // = 32 bytes

// ── Loaded types (Somnium-native, no gltf:: types) ────────────────────────

/// Raw RGBA8 image decoded from a glTF texture.
pub struct LoadedTexture {
    pub data: Vec<u8>, // row-major RGBA8 pixels
    pub width: u32,
    pub height: u32,
}

/// How a material's alpha is interpreted (glTF `alphaMode`).
///
/// The renderer routes `Blend` materials to a separate forward pass — the
/// visibility buffer stores one triangle per pixel and structurally cannot
/// represent see-through surfaces.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
pub enum AlphaMode {
    /// Alpha ignored; surface is fully opaque.
    #[default]
    Opaque,
    /// Fragments below `alpha_cutoff` are discarded, the rest are opaque.
    Mask,
    /// Alpha-blended against what is already in the frame buffer.
    Blend,
}

/// PBR metallic-roughness material. Texture indices reference `LoadedScene.textures`.
pub struct LoadedMaterial {
    /// Source material name, used for editable `.sommat` sibling naming.
    pub name: String,
    pub base_color: [f32; 4],
    pub roughness: f32,
    pub metallic: f32,
    pub albedo_map: Option<usize>,
    /// glTF `occlusionTexture` (red channel). Distinct from the
    /// metallic-roughness texture: the spec leaves *its* red channel
    /// undefined, so AO must never be read from it.
    pub occlusion_map: Option<usize>,
    pub normal_map: Option<usize>,
    pub metallic_roughness_map: Option<usize>,
    /// glTF `alphaMode`. Dropping this was why blended glass rendered as
    /// opaque grey panels.
    pub alpha_mode: AlphaMode,
    /// Threshold for [`AlphaMode::Mask`].
    pub alpha_cutoff: f32,
    /// How much light passes *through* the surface (Phase 24S).
    ///
    /// 0 is opaque. Leaves and grass blades are thin enough that a large
    /// fraction of the light hitting their far side comes out toward the
    /// viewer, which is why real foliage glows when backlit and looks flat and
    /// dead without it — no amount of correcting the albedo substitutes.
    pub transmission: f32,
    /// Phase 17E: this material is vegetation — a thin, cut-out card.
    ///
    /// Inferred from the same `*_alpha_*` sidecar convention that promotes it
    /// to MASK. Kept separate from `transmission` because glass is transmissive
    /// too and must not be treated as a leaf.
    pub foliage: bool,
    /// Light the surface emits on its own, linear RGB (Phase 24T).
    ///
    /// Carried in the same photometric scale as everything else since 24A, so
    /// a screen or a lit sign has to be given a real luminance rather than a
    /// number that happened to look right.
    pub emissive: [f32; 3],
    /// Multiplier from `KHR_materials_emissive_strength`.
    pub emissive_intensity: f32,
    /// Texture modulating `emissive`, if the material has one.
    pub emissive_map: Option<usize>,
    /// glTF `doubleSided` — blended geometry is usually thin and needs both faces.
    pub double_sided: bool,
}

/// A single mesh primitive (position + normal + UV geometry + triangle indices).
pub struct LoadedMesh {
    pub vertices: Vec<Vertex>,
    pub indices: Vec<u32>,
    /// MORROWIND-U. Per-vertex skin binding, when the primitive has `JOINTS_0`
    /// and `WEIGHTS_0`.
    ///
    /// A **parallel array rather than a wider `Vertex`**, and that is the whole
    /// design decision: `Vertex` is 32 bytes in `GeometryPool`'s shared buffer,
    /// which every pass in the renderer reads and which ray tracing reads
    /// positions straight out of (`geometry.rs:122`). Widening it to carry four
    /// joints and four weights would cost 24 bytes on **every** vertex in the
    /// world — terrain, foliage, props — to serve the handful that are skinned.
    ///
    /// `None` for the overwhelming majority of meshes, which is the point.
    pub skin: Option<somnium_anim::Skin>,
}

/// A scene node flattened to world-space. Mesh/material indices reference
/// the parallel `meshes` and `materials` vecs in `LoadedScene`.
pub struct SceneNode {
    pub name: String,
    pub mesh_index: Option<usize>,
    pub material_index: Option<usize>,
    pub transform: Mat4, // world-space (parent chain already multiplied)
}

/// The result of `load_gltf`. Everything the renderer needs to upload a scene.
#[derive(Default)]
pub struct LoadedScene {
    pub meshes: Vec<LoadedMesh>,
    pub materials: Vec<LoadedMaterial>,
    pub textures: Vec<LoadedTexture>,
    pub nodes: Vec<SceneNode>,
    /// MORROWIND-U. One per glTF `skin`, indexed by `SkeletonId`.
    ///
    /// Joints are **reordered** by `Skeleton::new` so every parent precedes its
    /// children, and every vertex joint index in `LoadedMesh::skin` has already
    /// been remapped to match. A caller never sees the authored order, which is
    /// deliberate: the invariant is worth nothing if it holds in the skeleton
    /// and not in the vertices that index it.
    pub skeletons: Vec<somnium_anim::Skeleton>,
}

// ── Public API ─────────────────────────────────────────────────────────────

/// Load a glTF 2.0 / GLB file and return a `LoadedScene`.
///
/// No `gltf::` types escape this function — callers only see Somnium types.
pub fn load_gltf(path: impl AsRef<Path>) -> Result<LoadedScene, String> {
    let path = path.as_ref();
    info!("Loading glTF: {:?}", path);

    let (document, buffers, images) =
        gltf::import(path).map_err(|e| format!("glTF import failed: {e}"))?;

    let mut scene = LoadedScene::default();

    // 1. Textures → RGBA8 -----------------------------------------------
    for img in &images {
        scene.textures.push(decode_to_rgba8(img));
    }

    // 1b. Sidecar alpha masks -------------------------------------------
    let base_dir = path.parent().unwrap_or_else(|| Path::new("."));
    let masked = attach_sidecar_alpha(&document, base_dir, &mut scene.textures);

    // Images whose filename marks them as ARM-packed (AO / Roughness / Metallic).
    let arm_packed: std::collections::HashSet<usize> = document
        .images()
        .filter(|img| match img.source() {
            gltf::image::Source::Uri { uri, .. } => uri.contains("_arm"),
            _ => false,
        })
        .map(|img| img.index())
        .collect();

    // 2. Materials -------------------------------------------------------
    for mat in document.materials() {
        let pbr = mat.pbr_metallic_roughness();
        scene.materials.push(LoadedMaterial {
            name: mat.name().unwrap_or("Material").to_string(),
            base_color: pbr.base_color_factor(),
            roughness: pbr.roughness_factor(),
            metallic: pbr.metallic_factor(),
            alpha_mode: match mat.alpha_mode() {
                gltf::material::AlphaMode::Opaque => AlphaMode::Opaque,
                gltf::material::AlphaMode::Mask => AlphaMode::Mask,
                gltf::material::AlphaMode::Blend => AlphaMode::Blend,
            },
            alpha_cutoff: mat.alpha_cutoff().unwrap_or(0.5),
            transmission: mat.transmission().map_or(0.0, |t| t.transmission_factor()),
            // Set below, where the sidecar cutout mask identifies vegetation.
            foliage: false,
            emissive: mat.emissive_factor(),
            emissive_intensity: mat.emissive_strength().unwrap_or(1.0),
            emissive_map: mat.emissive_texture().map(|t| t.texture().source().index()),
            double_sided: mat.double_sided(),
            albedo_map: pbr
                .base_color_texture()
                .map(|t| t.texture().source().index()),
            occlusion_map: mat
                .occlusion_texture()
                .map(|t| t.texture().source().index()),
            normal_map: mat.normal_texture().map(|t| t.texture().source().index()),
            metallic_roughness_map: pbr
                .metallic_roughness_texture()
                .map(|t| t.texture().source().index()),
        });
    }
    // ARM-packed textures carry occlusion in red, but glTF has no way to say
    // so: exporters that pack this way (Poly Haven's among them) simply leave
    // `occlusionTexture` unset and the AO channel goes unused. The `_arm`
    // filename is the convention that states the packing, so honour it — and
    // only it, because the spec leaves a plain metallic-roughness map's red
    // channel undefined and reading that would darken models to black.
    for m in &mut scene.materials {
        if m.occlusion_map.is_none()
            && m.metallic_roughness_map
                .is_some_and(|i| arm_packed.contains(&i))
        {
            m.occlusion_map = m.metallic_roughness_map;
        }
    }

    // A sidecar mask means the albedo is a cutout atlas, so the material has
    // to be alpha-tested even though the glTF called it opaque.
    for m in &mut scene.materials {
        if m.albedo_map.is_some_and(|i| masked.contains(&i)) && m.alpha_mode == AlphaMode::Opaque {
            m.alpha_mode = AlphaMode::Mask;
            // Foliage masks are near-binary; cutting at the midpoint keeps
            // blade edges crisp without eroding thin tips.
            m.alpha_cutoff = 0.5;
            m.double_sided = true;
            m.foliage = true;
            // A sidecar cutout mask means foliage: thin, and translucent.
            // The glTF carries no transmission factor for these assets, and
            // inferring it from the same convention is better than leaving
            // every leaf opaque.
            if m.transmission <= 0.0 {
                m.transmission = 0.5;
            }
        }
    }

    // Ensure there is always at least a default material at index 0.
    if scene.materials.is_empty() {
        scene.materials.push(LoadedMaterial {
            name: "Material".to_string(),
            base_color: [1.0, 1.0, 1.0, 1.0],
            roughness: 0.5,
            metallic: 0.0,
            albedo_map: None,
            occlusion_map: None,
            normal_map: None,
            metallic_roughness_map: None,
            alpha_mode: AlphaMode::Opaque,
            alpha_cutoff: 0.5,
            transmission: 0.0,
            foliage: false,
            emissive: [0.0; 3],
            emissive_intensity: 1.0,
            emissive_map: None,
            double_sided: false,
        });
    }

    // 2.5. Skins (MORROWIND-U) -------------------------------------------
    //
    // Before meshes, because a primitive's `JOINTS_0` indices refer to the
    // *authored* joint order and `Skeleton::new` reorders. The remap has to
    // exist before a vertex is read, or the invariant holds in the skeleton and
    // not in the thing that indexes it.
    //
    // glTF has no way to say which skin a *primitive* uses — a skin is on a
    // node. Somnium's `LoadedMesh` is per-primitive, so the association is made
    // in the node walk below and a primitive reachable from two nodes with
    // different skins takes the first, which is a case no exporter produces and
    // which would be ambiguous if one did.
    let mut joint_remaps: Vec<Vec<u16>> = Vec::new();
    for skin in document.skins() {
        let reader = skin.reader(|buf| Some(&buffers[buf.index()]));
        let joints: Vec<gltf::Node> = skin.joints().collect();

        // glTF gives a joint's parent implicitly, through the node hierarchy.
        // Invert it: a child's index in `joints` maps back to whichever joint
        // lists it as a child.
        let node_to_joint: HashMap<usize, u16> = joints
            .iter()
            .enumerate()
            .map(|(index, node)| (node.index(), index as u16))
            .collect();
        let mut parents = vec![somnium_anim::NO_PARENT; joints.len()];
        for (index, node) in joints.iter().enumerate() {
            for child in node.children() {
                if let Some(&child_joint) = node_to_joint.get(&child.index()) {
                    parents[child_joint as usize] = index as u16;
                }
            }
        }

        let inverse_bind: Vec<Mat4> = match reader.read_inverse_bind_matrices() {
            Some(matrices) => matrices.map(|m| Mat4::from_cols_array_2d(&m)).collect(),
            // The spec says an absent accessor means identity for every joint,
            // which is a skeleton already at its bind pose.
            None => vec![Mat4::IDENTITY; joints.len()],
        };

        let rest: Vec<somnium_anim::Transform> = joints
            .iter()
            .map(|node| {
                let (translation, rotation, scale) = node.transform().decomposed();
                somnium_anim::Transform {
                    translation: Vec3::from(translation),
                    rotation: glam::Quat::from_array(rotation),
                    scale: Vec3::from(scale),
                }
            })
            .collect();

        let names: Vec<String> = joints
            .iter()
            .enumerate()
            .map(|(index, node)| {
                node.name().unwrap_or("joint").to_string()
                    + &{
                        // Names are not unique in glTF and `Skeleton::find` is by name,
                        // so a duplicate would make lookup silently pick the first.
                        // Suffixing only on collision would be prettier and would also
                        // make the name depend on file order; suffixing never is worse.
                        // This is the compromise: index-suffixed, always, and stated.
                        format!("#{index}")
                    }
            })
            .collect();

        let id = somnium_anim::SkeletonId(scene.skeletons.len() as u32);
        match somnium_anim::Skeleton::new(id, names, parents, inverse_bind, rest) {
            Some((skeleton, remap)) => {
                scene.skeletons.push(skeleton);
                joint_remaps.push(remap);
            }
            None => {
                return Err(format!(
                    "Skin '{}' has a malformed joint hierarchy (a cycle, or a parent out of range)",
                    skin.name().unwrap_or("?")
                ));
            }
        }
    }

    // Which skin each mesh is used with, from the node hierarchy.
    let mut mesh_skin: HashMap<usize, usize> = HashMap::new();
    for node in document.nodes() {
        if let (Some(mesh), Some(skin)) = (node.mesh(), node.skin()) {
            mesh_skin.entry(mesh.index()).or_insert(skin.index());
        }
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

            let normals: Option<Vec<[f32; 3]>> = reader.read_normals().map(|n| n.collect());
            let uvs: Option<Vec<[f32; 2]>> =
                reader.read_tex_coords(0).map(|u| u.into_f32().collect());

            let mut vertices: Vec<Vertex> = positions
                .iter()
                .enumerate()
                .map(|(i, &pos)| Vertex {
                    position: pos,
                    normal: normals
                        .as_ref()
                        .and_then(|n| n.get(i).copied())
                        .unwrap_or([0.0, 1.0, 0.0]),
                    uv: uvs
                        .as_ref()
                        .and_then(|u| u.get(i).copied())
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

            // MORROWIND-U. Skin binding, if this primitive has one *and* the
            // mesh is used by a node with a skin. Both halves are required:
            // JOINTS_0 without a skin is data with nothing to index into.
            let skin = mesh_skin.get(&mesh.index()).and_then(|&skin_index| {
                let remap = joint_remaps.get(skin_index)?;
                let joints: Vec<[u16; 4]> = reader.read_joints(0)?.into_u16().collect();
                let weights: Vec<[f32; 4]> = reader.read_weights(0)?.into_f32().collect();
                let bindings = (0..vertices.len())
                    .map(|i| {
                        let (Some(j), Some(w)) = (joints.get(i), weights.get(i)) else {
                            // A vertex past the end of either accessor. Bound to
                            // the root rather than to nothing, per
                            // `SkinBinding::UNSKINNED`: zero weights put the
                            // vertex at the origin and read as a spike.
                            return somnium_anim::SkinBinding::UNSKINNED;
                        };
                        let influences: Vec<(u16, f32)> = (0..4)
                            .filter_map(|k| {
                                let authored = j[k] as usize;
                                // A joint index past the skeleton is a broken
                                // file; dropping the influence is better than
                                // reading past the palette on the GPU.
                                remap.get(authored).map(|&mapped| (mapped, w[k]))
                            })
                            .collect();
                        somnium_anim::SkinBinding::from_influences(&influences)
                    })
                    .collect();
                Some(somnium_anim::Skin {
                    skeleton: somnium_anim::SkeletonId(skin_index as u32),
                    bindings,
                })
            });

            let loaded_idx = scene.meshes.len();
            prim_to_loaded.insert((mesh.index(), prim_idx), loaded_idx);

            let _ = prim_count; // used above for naming
            scene.meshes.push(LoadedMesh {
                vertices,
                indices,
                skin,
            });
        }
    }

    // 4. Scene graph → flat world-space nodes ----------------------------
    let gltf_scene = document
        .default_scene()
        .or_else(|| document.scenes().next());
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
    node: &gltf::Node,
    parent_xform: Mat4,
    prim_to_loaded: &HashMap<(usize, usize), usize>,
    out: &mut Vec<SceneNode>,
) {
    let local = Mat4::from_cols_array_2d(&node.transform().matrix());
    let world = parent_xform * local;
    let node_name = node.name().unwrap_or("Node").to_string();

    if let Some(mesh) = node.mesh() {
        let prim_count = mesh.primitives().count();
        for (prim_idx, primitive) in mesh.primitives().enumerate() {
            let mesh_index = prim_to_loaded.get(&(mesh.index(), prim_idx)).copied();
            let material_index = primitive.material().index();
            let name = if prim_count == 1 {
                node_name.clone()
            } else {
                format!("{node_name}_{prim_idx}")
            };
            out.push(SceneNode {
                name,
                mesh_index,
                material_index,
                transform: world,
            });
        }
    }

    for child in node.children() {
        collect_nodes(&child, world, prim_to_loaded, out);
    }
}

/// Fold sibling `*_alpha_*` masks into their albedo texture's alpha channel.
///
/// Poly Haven (and most scanned-foliage libraries) ship vegetation as cutout
/// cards: the diffuse atlas holds blade or leaf shapes on a black background,
/// and the shape itself lives in a *separate* alpha map. Their glTF export
/// references only diffuse/normal/ARM, so a loader that trusts the glTF renders
/// the black background as if it were the plant — which reads as dark, blue-ish
/// blobs once ambient light is the only thing left to reflect.
///
/// The pairing is by filename: `X_diff_2k.jpg` takes its mask from
/// `X_alpha_2k.png`. That is a convention rather than a standard, so a missing
/// or unreadable sidecar is not an error — the texture is simply left alone.
///
/// Returns the set of texture indices that gained a mask, so callers can switch
/// the corresponding materials to alpha testing.
fn attach_sidecar_alpha(
    document: &gltf::Document,
    base_dir: &Path,
    textures: &mut [LoadedTexture],
) -> std::collections::HashSet<usize> {
    let mut masked = std::collections::HashSet::new();

    for image in document.images() {
        let gltf::image::Source::Uri { uri, .. } = image.source() else {
            continue; // embedded texture: no sidecar to find
        };
        let Some(mask_path) = sidecar_alpha_path(base_dir, uri) else {
            continue;
        };
        let Some(tex) = textures.get_mut(image.index()) else {
            continue;
        };

        let mask = match image::open(&mask_path) {
            Ok(m) => m.into_luma8(),
            Err(e) => {
                warn!("Alpha sidecar {mask_path:?} failed to decode: {e}");
                continue;
            }
        };
        if mask.width() != tex.width || mask.height() != tex.height {
            warn!(
                "Alpha sidecar {:?} is {}x{} but its albedo is {}x{}; skipping",
                mask_path,
                mask.width(),
                mask.height(),
                tex.width,
                tex.height,
            );
            continue;
        }

        for (texel, m) in tex.data.chunks_exact_mut(4).zip(mask.pixels()) {
            texel[3] = m.0[0];
        }
        masked.insert(image.index());
        info!(
            "Applied alpha sidecar {:?}",
            mask_path.file_name().unwrap_or_default()
        );
    }

    masked
}

/// Resolve `<dir>/<stem-with-_diff_-swapped-for-_alpha_>.png`, if it exists.
///
/// Split on `_diff` rather than the whole `_diff_2k` so the same rule survives
/// other resolutions (`_diff_4k`, `_diff_1k`) and suffixed variants.
fn sidecar_alpha_path(base_dir: &Path, uri: &str) -> Option<std::path::PathBuf> {
    // glTF URIs are percent-encoded and always use forward slashes.
    let uri = uri.replace("%20", " ");
    let (dir, file) = match uri.rsplit_once('/') {
        Some((d, f)) => (base_dir.join(d), f.to_string()),
        None => (base_dir.to_path_buf(), uri.clone()),
    };
    let (before, after) = file.split_once("_diff")?;
    let after = after.rsplit_once('.').map_or("", |(stem, _)| stem);
    let candidate = dir.join(format!("{before}_alpha{after}.png"));
    candidate.exists().then_some(candidate)
}

/// Convert any gltf image format to tightly-packed RGBA8.
fn decode_to_rgba8(data: &gltf::image::Data) -> LoadedTexture {
    use gltf::image::Format;
    let rgba: Vec<u8> = match data.format {
        Format::R8G8B8A8 => data.pixels.clone(),
        Format::R8G8B8 => data
            .pixels
            .chunks_exact(3)
            .flat_map(|rgb| [rgb[0], rgb[1], rgb[2], 255])
            .collect(),
        Format::R8 => data.pixels.iter().flat_map(|&r| [r, r, r, 255]).collect(),
        Format::R8G8 => data
            .pixels
            .chunks_exact(2)
            .flat_map(|rg| [rg[0], rg[1], 0, 255])
            .collect(),
        _ => {
            warn!(
                "Unsupported texture format {:?}; using magenta placeholder",
                data.format
            );
            [255u8, 0, 255, 255]
                .iter()
                .cycle()
                .take((data.width * data.height * 4) as usize)
                .copied()
                .collect()
        }
    };
    LoadedTexture {
        data: rgba,
        width: data.width,
        height: data.height,
    }
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
        0, 1, 2, 2, 3, 0, 4, 5, 6, 6, 7, 4, 8, 9, 10, 10, 11, 8, 12, 13, 14, 14, 15, 12, 16, 17,
        18, 18, 19, 16, 20, 21, 22, 22, 23, 20,
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
                normal: [nx, ny, nz],
                uv: [seg as f32 / segments as f32, ring as f32 / rings as f32],
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
                normal: [0.0, 1.0, 0.0],
                uv: [ix as f32 / divs as f32, iz as f32 / divs as f32],
            });
        }
    }

    let row = divs + 1;
    for iz in 0..divs {
        for ix in 0..divs {
            let base = iz * row + ix;
            indices.extend_from_slice(&[
                base,
                base + row,
                base + 1,
                base + 1,
                base + row,
                base + row + 1,
            ]);
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
        vertices.push(Vertex {
            position: [c * radius, -h, s * radius],
            normal: [c, 0.0, s],
            uv: [u, 1.0],
        });
        vertices.push(Vertex {
            position: [c * radius, h, s * radius],
            normal: [c, 0.0, s],
            uv: [u, 0.0],
        });
    }
    for seg in 0..segments {
        let b = side_base + seg * 2;
        indices.extend_from_slice(&[b, b + 1, b + 2, b + 2, b + 1, b + 3]);
    }

    // Top cap
    let top_centre = vertices.len() as u32;
    vertices.push(Vertex {
        position: [0.0, h, 0.0],
        normal: [0.0, 1.0, 0.0],
        uv: [0.5, 0.5],
    });
    let top_ring_start = vertices.len() as u32;
    for seg in 0..segments {
        let theta = 2.0 * std::f32::consts::PI * seg as f32 / segments as f32;
        let c = theta.cos();
        let s = theta.sin();
        vertices.push(Vertex {
            position: [c * radius, h, s * radius],
            normal: [0.0, 1.0, 0.0],
            uv: [c * 0.5 + 0.5, s * 0.5 + 0.5],
        });
    }
    for seg in 0..segments {
        let a = top_ring_start + seg;
        let b = top_ring_start + (seg + 1) % segments;
        indices.extend_from_slice(&[top_centre, a, b]);
    }

    // Bottom cap
    let bot_centre = vertices.len() as u32;
    vertices.push(Vertex {
        position: [0.0, -h, 0.0],
        normal: [0.0, -1.0, 0.0],
        uv: [0.5, 0.5],
    });
    let bot_ring_start = vertices.len() as u32;
    for seg in 0..segments {
        let theta = 2.0 * std::f32::consts::PI * seg as f32 / segments as f32;
        let c = theta.cos();
        let s = theta.sin();
        vertices.push(Vertex {
            position: [c * radius, -h, s * radius],
            normal: [0.0, -1.0, 0.0],
            uv: [c * 0.5 + 0.5, s * 0.5 + 0.5],
        });
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

/// A grass/shrub tuft: `blades` tapered three-sided spikes fanning out from a
/// common base (Phase 17A).
///
/// Solid geometry rather than the usual alpha-tested crossed billboards,
/// because the visibility pass culls back faces — a flat quad would vanish from
/// one side — and the engine imports `alphaMode: MASK` without cutting it out
/// in the shader yet. A three-sided prism reads as a blade from every angle and
/// needs no alpha at all.
///
/// Unit-sized: roughly 1 unit tall and 0.6 wide, so instance scale is the only
/// size control.
pub fn generate_foliage_tuft(blades: u32, seed: u32) -> (Vec<Vertex>, Vec<u32>) {
    let blades = blades.clamp(1, 32);
    let mut vertices = Vec::new();
    let mut indices: Vec<u32> = Vec::new();

    // Cheap deterministic jitter so the blades of one tuft differ from each
    // other; the caller varies `seed` only if it wants a second tuft variant.
    let rand = |n: u32| {
        let mut h = n.wrapping_mul(0x9E37_79B9) ^ seed.wrapping_mul(0x85EB_CA6B);
        h ^= h >> 15;
        h = h.wrapping_mul(0x2545_F491);
        h ^= h >> 13;
        (h >> 8) as f32 / 16_777_216.0
    };

    for b in 0..blades {
        // Fan the blades evenly, then jitter so they do not look combed.
        let angle = (b as f32 / blades as f32 + rand(b * 7) * 0.15) * std::f32::consts::TAU;
        let lean = 0.18 + rand(b * 13) * 0.30;
        let height = 0.65 + rand(b * 17) * 0.45;
        let half_w = 0.045 + rand(b * 23) * 0.025;

        let dir = [angle.cos(), angle.sin()];
        // Perpendicular in XZ, so the blade has width across its lean.
        let side = [-dir[1], dir[0]];

        let tip = [dir[0] * lean * height, height, dir[1] * lean * height];
        let base = vertices.len() as u32;

        // Three base corners around the origin, one tip: a tapered spike.
        for k in 0..3 {
            let a = k as f32 / 3.0 * std::f32::consts::TAU;
            // Unit outward direction for this corner, in world XZ.
            let ox = side[0] * a.cos() + dir[0] * a.sin();
            let oz = side[1] * a.cos() + dir[1] * a.sin();
            // Lean the normal outward rather than nearly straight up. With an
            // almost vertical normal every blade catches identical sunlight and
            // the tuft reads as a flat pale smudge; tilting it out gives each
            // face its own shading and the clump some depth.
            let n = [ox, 0.55, oz];
            let len = (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt().max(1e-6);
            vertices.push(Vertex {
                position: [ox * half_w, 0.0, oz * half_w],
                normal: [n[0] / len, n[1] / len, n[2] / len],
                uv: [k as f32 / 3.0, 1.0],
            });
        }
        vertices.push(Vertex {
            position: tip,
            normal: [0.0, 1.0, 0.0],
            uv: [0.5, 0.0],
        });

        // Three side faces, wound counter-clockwise seen from outside so the
        // back-face cull keeps them.
        for k in 0..3u32 {
            let a = base + k;
            let c = base + (k + 1) % 3;
            indices.extend_from_slice(&[a, c, base + 3]);
        }
    }

    (vertices, indices)
}

#[cfg(test)]
mod foliage_tuft_tests {
    use super::*;

    #[test]
    fn a_tuft_has_one_spike_per_blade() {
        let (v, i) = generate_foliage_tuft(5, 1);
        assert_eq!(v.len(), 5 * 4, "3 base corners + 1 tip per blade");
        assert_eq!(i.len(), 5 * 3 * 3, "3 triangles per blade");
    }

    #[test]
    fn every_index_addresses_a_real_vertex() {
        let (v, i) = generate_foliage_tuft(7, 3);
        assert!(i.iter().all(|&x| (x as usize) < v.len()));
    }

    #[test]
    fn the_tuft_sits_on_the_origin_and_grows_upward() {
        // The scatter places instances at ground height, so a tuft whose base
        // was not at y = 0 would float or sink.
        let (v, _) = generate_foliage_tuft(6, 2);
        let min_y = v.iter().fold(f32::MAX, |m, x| m.min(x.position[1]));
        let max_y = v.iter().fold(f32::MIN, |m, x| m.max(x.position[1]));
        assert!(min_y.abs() < 1e-6, "base at {min_y}");
        assert!((0.6..1.6).contains(&max_y), "height {max_y}");
    }

    #[test]
    fn normals_are_unit_length() {
        let (v, _) = generate_foliage_tuft(6, 2);
        for x in &v {
            let n = x.normal;
            let len = (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt();
            assert!((len - 1.0).abs() < 1e-4, "normal length {len}");
        }
    }

    #[test]
    fn the_blade_count_is_clamped_rather_than_producing_nothing() {
        assert_eq!(generate_foliage_tuft(0, 1).0.len(), 4);
        assert_eq!(generate_foliage_tuft(1000, 1).0.len(), 32 * 4);
    }

    #[test]
    fn the_same_seed_gives_the_same_tuft() {
        assert_eq!(
            generate_foliage_tuft(6, 9).0.len(),
            generate_foliage_tuft(6, 9).0.len()
        );
        let a = generate_foliage_tuft(6, 9).0;
        let b = generate_foliage_tuft(6, 9).0;
        assert!(a.iter().zip(&b).all(|(x, y)| x.position == y.position));
    }
}
