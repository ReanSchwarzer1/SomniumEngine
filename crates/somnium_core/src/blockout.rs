//! MORROWIND-P: schema-authored blockout geometry with ordinary mesh output.
//!
//! The authored description remains independent of renderer allocations. Like
//! TrenchBroom's reflected entity classes, the engine's existing component
//! schema supplies both Details and scene persistence; no brush-only schema.

use glam::Vec3;
use somnium_asset::{LoadedMesh, Vertex};
use somnium_ecs::{Component, World, component_schema, reflect::TypeRegistry};

/// Parametric local-space level geometry. Shape: 0 box, 1 ramp, 2 stairs,
/// 3 cylinder. Positive size is full width, height and depth in metres.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BlockoutComponent {
    /// 0 box, 1 ramp, 2 stairs, or 3 cylinder.
    pub shape: u32,
    /// Full local dimensions in metres, before the entity transform.
    pub size: Vec3,
    /// Stair count or cylinder sides; ignored for a box or ramp.
    pub segments: u32,
}

impl Default for BlockoutComponent {
    fn default() -> Self {
        Self {
            shape: 0,
            size: Vec3::splat(2.0),
            segments: 12,
        }
    }
}
impl Component for BlockoutComponent {}

impl BlockoutComponent {
    /// Generate a validated, bounded mesh usable by both renderer and export.
    pub fn mesh(self) -> Result<LoadedMesh, String> {
        if self.shape > 3
            || !self.size.is_finite()
            || self.size.min_element() <= 0.0
            || self.size.max_element() > 100_000.0
        {
            return Err(
                "Blockout requires shape 0..3 and finite positive dimensions <= 100000".into(),
            );
        }
        if (self.shape == 2 && !(1..=256).contains(&self.segments))
            || (self.shape == 3 && !(3..=256).contains(&self.segments))
        {
            return Err("Stairs require 1..256 steps; cylinders require 3..256 sides".into());
        }
        let (mut vertices, mut indices) = (Vec::new(), Vec::new());
        match self.shape {
            0 | 3 => {
                (vertices, indices) = if self.shape == 0 {
                    somnium_asset::generate_cube(1.0)
                } else {
                    somnium_asset::generate_cylinder(0.5, 1.0, self.segments)
                };
                for vertex in &mut vertices {
                    vertex.position = (Vec3::from(vertex.position) * self.size).to_array();
                    vertex.normal = (Vec3::from(vertex.normal) / self.size)
                        .normalize()
                        .to_array();
                }
            }
            1 => {
                let h = self.size * 0.5;
                let p = [
                    Vec3::new(-h.x, -h.y, -h.z),
                    Vec3::new(h.x, -h.y, -h.z),
                    Vec3::new(-h.x, -h.y, h.z),
                    Vec3::new(h.x, -h.y, h.z),
                    Vec3::new(-h.x, h.y, h.z),
                    Vec3::new(h.x, h.y, h.z),
                ];
                for [a, b, c] in [
                    [0, 1, 2],
                    [1, 3, 2],
                    [2, 3, 4],
                    [3, 5, 4],
                    [0, 4, 1],
                    [1, 4, 5],
                    [0, 2, 4],
                    [1, 5, 3],
                ] {
                    triangle(&mut vertices, &mut indices, p[a], p[b], p[c]);
                }
            }
            2 => {
                for step in 0..self.segments {
                    let height = self.size.y * (step + 1) as f32 / self.segments as f32;
                    let depth = self.size.z / self.segments as f32;
                    let offset = Vec3::new(
                        0.0,
                        (height - self.size.y) * 0.5,
                        -self.size.z * 0.5 + depth * (step as f32 + 0.5),
                    );
                    let (mut verts, inds) = somnium_asset::generate_cube(1.0);
                    let base = vertices.len() as u32;
                    for vertex in &mut verts {
                        vertex.position = (Vec3::from(vertex.position)
                            * Vec3::new(self.size.x, height, depth)
                            + offset)
                            .to_array();
                    }
                    vertices.extend(verts);
                    indices.extend(inds.into_iter().map(|index| index + base));
                }
            }
            _ => unreachable!(),
        }
        Ok(LoadedMesh {
            vertices,
            indices,
            skin: None,
        })
    }

    /// Convert into a standard glTF binary mesh asset, accepted by the same
    /// Content importer and cooker as artist-authored geometry.
    pub fn write_glb(self, path: &std::path::Path) -> Result<(), String> {
        let mesh = self.mesh()?;
        let mut binary = Vec::new();
        for vertex in &mesh.vertices {
            for value in vertex
                .position
                .into_iter()
                .chain(vertex.normal)
                .chain(vertex.uv)
            {
                binary.extend_from_slice(&value.to_le_bytes());
            }
        }
        let index_offset = binary.len();
        for index in &mesh.indices {
            binary.extend_from_slice(&index.to_le_bytes());
        }
        let min = mesh
            .vertices
            .iter()
            .fold(Vec3::splat(f32::INFINITY), |v, p| v.min(p.position.into()));
        let max = mesh
            .vertices
            .iter()
            .fold(Vec3::splat(f32::NEG_INFINITY), |v, p| {
                v.max(p.position.into())
            });
        let json = serde_json::json!({
            "asset":{"version":"2.0","generator":"Somnium Blockout"},
            "buffers":[{"byteLength":binary.len()}],
            "bufferViews":[{"buffer":0,"byteOffset":0,"byteLength":index_offset,"byteStride":32,"target":34962},
                {"buffer":0,"byteOffset":index_offset,"byteLength":mesh.indices.len()*4,"target":34963}],
            "accessors":[{"bufferView":0,"byteOffset":0,"componentType":5126,"count":mesh.vertices.len(),"type":"VEC3","min":min.to_array(),"max":max.to_array()},
                {"bufferView":0,"byteOffset":12,"componentType":5126,"count":mesh.vertices.len(),"type":"VEC3"},
                {"bufferView":0,"byteOffset":24,"componentType":5126,"count":mesh.vertices.len(),"type":"VEC2"},
                {"bufferView":1,"componentType":5125,"count":mesh.indices.len(),"type":"SCALAR"}],
            "meshes":[{"primitives":[{"attributes":{"POSITION":0,"NORMAL":1,"TEXCOORD_0":2},"indices":3}]}],
            "nodes":[{"mesh":0}],"scenes":[{"nodes":[0]}],"scene":0
        });
        let mut json = serde_json::to_vec(&json).map_err(|e| e.to_string())?;
        while json.len() % 4 != 0 {
            json.push(b' ');
        }
        let mut bytes = Vec::new();
        for value in [
            0x4654_6c67_u32,
            2,
            (28 + json.len() + binary.len()) as u32,
            json.len() as u32,
            0x4e4f_534a,
        ] {
            bytes.extend_from_slice(&value.to_le_bytes());
        }
        bytes.extend(json);
        bytes.extend_from_slice(&(binary.len() as u32).to_le_bytes());
        bytes.extend_from_slice(&0x004e_4942_u32.to_le_bytes());
        bytes.extend(binary);
        std::fs::write(path, bytes).map_err(|e| e.to_string())
    }
}

fn triangle(vertices: &mut Vec<Vertex>, indices: &mut Vec<u32>, a: Vec3, b: Vec3, c: Vec3) {
    let normal = (b - a).cross(c - a).normalize().to_array();
    for (p, uv) in [(a, [0.0, 0.0]), (b, [1.0, 0.0]), (c, [0.0, 1.0])] {
        indices.push(vertices.len() as u32);
        vertices.push(Vertex {
            position: p.to_array(),
            normal,
            uv,
        });
    }
}

pub(crate) fn register(registry: &mut TypeRegistry) {
    let mut schema = component_schema! {
        BlockoutComponent as "somnium.Blockout", display "Blockout", version 1,
        fields {
            shape { min: 0.0, max: 3.0, doc: "Primitive shape." },
            size { doc: "Full local width, height and depth in metres." },
            segments { doc: "Stairs: 1–256 steps. Cylinder: 3–256 sides." },
        }
    };
    if let Some(field) = schema.fields.iter_mut().find(|field| field.name == "shape") {
        field.ty = somnium_ecs::reflect::FieldType::Enum(&["Box", "Ramp", "Stairs", "Cylinder"]);
    }
    registry.register(schema);
}

#[derive(Clone, Copy)]
struct Uploaded(BlockoutComponent, u32);
impl Component for Uploaded {}

/// Synchronize changed descriptions only. The derived upload marker is never
/// registered or saved, so scene reload rebuilds renderer state automatically.
pub(crate) fn sync(
    world: &mut World,
    renderer: &mut somnium_renderer::SomniumRenderer,
    context: &somnium_renderer::RenderContext,
) {
    let key = |shape: BlockoutComponent, material: u32| {
        (
            shape.shape,
            shape.size.to_array().map(f32::to_bits),
            shape.segments,
            material,
        )
    };
    let dirty: Vec<_> = world
        .entities()
        .filter_map(|entity| {
            let blockout = *world.get::<BlockoutComponent>(entity)?;
            let material = world
                .get::<crate::MaterialComponent>(entity)
                .map_or(0, |m| m.runtime_id);
            (world
                .get::<Uploaded>(entity)
                .is_none_or(|old| old.0 != blockout || old.1 != material)
                || world.get::<crate::MeshComponent>(entity).is_none())
            .then_some((entity, blockout, material))
        })
        .collect();
    if dirty.is_empty() {
        return;
    }
    // Scatter bakes may contain thousands of identical blockouts. Reuse the
    // same immutable allocation rather than uploading every instance again.
    let mut allocations: std::collections::BTreeMap<_, _> = world
        .entities()
        .filter_map(|entity| {
            let uploaded = world.get::<Uploaded>(entity)?;
            Some((
                key(uploaded.0, uploaded.1),
                *world.get::<crate::MeshComponent>(entity)?,
            ))
        })
        .collect();
    for (entity, blockout, material) in dirty {
        if let Some(mesh) = allocations.get(&key(blockout, material)).copied() {
            let _ = world.insert_component(entity, mesh);
            let _ = world.insert_component(entity, Uploaded(blockout, material));
            continue;
        }
        let Ok(mesh) = blockout.mesh() else {
            continue;
        };
        let upload =
            renderer
                .geometry
                .upload_mesh(&context.queue, &mesh.vertices, &mesh.indices, material);
        let mesh = crate::MeshComponent {
            vertex_offset: upload.vertex_offset,
            index_offset: upload.index_offset,
            index_count: upload.index_count,
        };
        allocations.insert(key(blockout, material), mesh);
        let _ = world.insert_component(entity, mesh);
        let _ = world.insert_component(entity, Uploaded(blockout, material));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shapes_have_finite_outward_faces_and_expected_bounds() {
        for shape in 0..4 {
            let blockout = BlockoutComponent {
                shape,
                size: Vec3::new(6.0, 4.0, 8.0),
                segments: 12,
            };
            let mesh = blockout.mesh().unwrap();
            assert!(mesh.indices.len() >= 24);
            for tri in mesh.indices.chunks_exact(3) {
                let a = &mesh.vertices[tri[0] as usize];
                let b = Vec3::from(mesh.vertices[tri[1] as usize].position);
                let c = Vec3::from(mesh.vertices[tri[2] as usize].position);
                let normal = Vec3::from(a.normal);
                assert!(normal.is_finite() && (normal.length() - 1.0).abs() < 1e-5);
                assert!(
                    (b - Vec3::from(a.position))
                        .cross(c - Vec3::from(a.position))
                        .dot(normal)
                        > 0.0
                );
            }
            for vertex in mesh.vertices {
                assert!(
                    (Vec3::from(vertex.position).abs() - blockout.size * 0.5).max_element() < 1e-4
                );
            }
        }
    }

    #[test]
    fn exported_glb_imports_through_the_standard_asset_loader() {
        let path =
            std::env::temp_dir().join(format!("somnium-blockout-{}.glb", std::process::id()));
        let blockout = BlockoutComponent {
            shape: 2,
            ..Default::default()
        };
        blockout.write_glb(&path).unwrap();
        let scene = somnium_asset::load_gltf(&path).unwrap();
        assert_eq!(scene.meshes[0].indices, blockout.mesh().unwrap().indices);
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn invalid_authoring_is_rejected_before_allocation() {
        assert!(
            BlockoutComponent {
                segments: u32::MAX,
                shape: 2,
                ..Default::default()
            }
            .mesh()
            .is_err()
        );
        assert!(
            BlockoutComponent {
                size: Vec3::splat(f32::NAN),
                ..Default::default()
            }
            .mesh()
            .is_err()
        );
    }

    #[test]
    fn scene_roundtrip_and_delete_snapshot_keep_authored_blockout() {
        let mut world = World::new();
        let blockout = BlockoutComponent {
            shape: 1,
            ..Default::default()
        };
        let entity = world.spawn((crate::Transform::default(), blockout));
        let id = world.ensure_persistent_id(entity).unwrap();
        let snapshot = crate::editor_commands::EntitySnapshot::capture(&world, entity);
        world.despawn(entity);
        let restored = snapshot.respawn(&mut world);
        assert_eq!(world.persistent_id(restored), Some(id));
        assert_eq!(world.get::<BlockoutComponent>(restored), Some(&blockout));
        let registry = crate::reflect_registry::component_registry();
        let document = crate::scene_schema::scene_to_json(&mut world, &registry);
        let mut loaded = World::new();
        crate::scene_schema::scene_from_json(&mut loaded, &registry, &document).unwrap();
        let restored = loaded.entity_by_persistent_id(id).unwrap();
        assert_eq!(loaded.get::<BlockoutComponent>(restored), Some(&blockout));
        assert_eq!(
            loaded
                .get::<BlockoutComponent>(restored)
                .unwrap()
                .mesh()
                .unwrap()
                .indices,
            blockout.mesh().unwrap().indices
        );
    }
}
