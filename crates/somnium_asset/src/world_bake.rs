//! Deterministic per-cell HLOD and octahedral impostor baking.

use crate::{database::AssetId, Vertex};
use glam::{Mat4, Vec3};
use serde::{Deserialize, Serialize};

/// One authored mesh instance entering the cell HLOD cook.
#[derive(Clone, Debug)]
pub struct HlodInstance {
    /// Source asset dependency.
    pub asset: AssetId,
    /// Source vertices.
    pub vertices: Vec<Vertex>,
    /// Triangle indices.
    pub indices: Vec<u32>,
    /// Authored local-to-cell transform.
    pub transform: Mat4,
    /// Representative linear base colour.
    pub base_color: [f32; 4],
}

/// Cooked merged proxy mesh and material.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct HlodProxy {
    /// Versioned bake format.
    pub version: u32,
    /// Interleaved position, normal and UV floats.
    pub vertices: Vec<[f32; 8]>,
    /// Merged triangle indices.
    pub indices: Vec<u32>,
    /// Area-weighted merged linear colour.
    pub base_color: [f32; 4],
    /// Sorted source dependencies.
    pub dependencies: Vec<AssetId>,
}

/// Merge cell geometry and retain at most `triangle_budget` triangles.
pub fn bake_hlod(instances: &[HlodInstance], triangle_budget: usize) -> Result<HlodProxy, String> {
    if triangle_budget == 0 {
        return Err("HLOD triangle budget must be non-zero".into());
    }
    let mut vertices = Vec::new();
    let mut triangles = Vec::new();
    let mut colour_sum = [0.0_f64; 4];
    let mut colour_weight = 0.0_f64;
    let mut dependencies: Vec<_> = instances.iter().map(|instance| instance.asset).collect();
    dependencies.sort_unstable();
    dependencies.dedup();
    for instance in instances {
        if instance.indices.len() % 3 != 0 {
            return Err("HLOD source index count is not triangular".into());
        }
        let base = u32::try_from(vertices.len()).map_err(|_| "HLOD vertex overflow")?;
        let normal_matrix = instance.transform.inverse().transpose();
        vertices.extend(instance.vertices.iter().map(|vertex| {
            let position = instance
                .transform
                .transform_point3(Vec3::from_array(vertex.position));
            let normal = normal_matrix
                .transform_vector3(Vec3::from_array(vertex.normal))
                .normalize_or_zero();
            [
                position.x,
                position.y,
                position.z,
                normal.x,
                normal.y,
                normal.z,
                vertex.uv[0],
                vertex.uv[1],
            ]
        }));
        for triangle in instance.indices.chunks_exact(3) {
            if triangle
                .iter()
                .any(|index| *index as usize >= instance.vertices.len())
            {
                return Err("HLOD source index is out of range".into());
            }
            triangles.push([base + triangle[0], base + triangle[1], base + triangle[2]]);
        }
        let weight = instance.indices.len().max(3) as f64 / 3.0;
        for (sum, channel) in colour_sum.iter_mut().zip(instance.base_color) {
            *sum += f64::from(channel) * weight;
        }
        colour_weight += weight;
    }
    if triangles.len() > triangle_budget {
        let source_count = triangles.len();
        triangles = (0..triangle_budget)
            .map(|slot| triangles[slot * source_count / triangle_budget])
            .collect();
    }
    let base_color = if colour_weight > 0.0 {
        colour_sum.map(|channel| (channel / colour_weight) as f32)
    } else {
        [1.0; 4]
    };
    Ok(HlodProxy {
        version: 1,
        vertices,
        indices: triangles.into_iter().flatten().collect(),
        base_color,
        dependencies,
    })
}

/// One square RGBA view captured by the offline renderer.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ImpostorView {
    /// Unit view direction encoded into atlas order.
    pub direction: [i16; 3],
    /// Square RGBA pixels.
    pub rgba: Vec<u8>,
}

/// Deterministic octahedral-view atlas.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ImpostorAtlas {
    /// Pixels per captured view edge.
    pub tile_size: u32,
    /// Tiles per atlas edge.
    pub grid_size: u32,
    /// Row-major RGBA atlas.
    pub rgba: Vec<u8>,
    /// Sorted octahedral direction keys matching tiles.
    pub directions: Vec<[i16; 2]>,
}

/// Project a direction onto a signed-normalized octahedron.
#[must_use]
pub fn oct_encode(direction: Vec3) -> [i16; 2] {
    let direction = direction.normalize_or_zero();
    let denominator = direction.x.abs() + direction.y.abs() + direction.z.abs();
    if denominator == 0.0 {
        return [0; 2];
    }
    let mut p = direction / denominator;
    if p.z < 0.0 {
        let old_x = p.x;
        p.x = (1.0 - p.y.abs()) * old_x.signum();
        p.y = (1.0 - old_x.abs()) * p.y.signum();
    }
    [
        (p.x.clamp(-1.0, 1.0) * f32::from(i16::MAX)).round() as i16,
        (p.y.clamp(-1.0, 1.0) * f32::from(i16::MAX)).round() as i16,
    ]
}

/// Pack offline captures by their octahedral key.
pub fn bake_impostor(views: &[ImpostorView], tile_size: u32) -> Result<ImpostorAtlas, String> {
    if views.is_empty() || tile_size == 0 {
        return Err("impostor bake needs views and a non-zero tile size".into());
    }
    let tile_bytes = tile_size as usize * tile_size as usize * 4;
    if views.iter().any(|view| view.rgba.len() != tile_bytes) {
        return Err("impostor capture dimensions disagree".into());
    }
    let mut ordered: Vec<_> = views
        .iter()
        .map(|view| {
            let direction = Vec3::new(
                f32::from(view.direction[0]),
                f32::from(view.direction[1]),
                f32::from(view.direction[2]),
            );
            (oct_encode(direction), view)
        })
        .collect();
    ordered.sort_by_key(|(key, _)| *key);
    if !ordered.windows(2).all(|pair| pair[0].0 < pair[1].0) {
        return Err("impostor views contain duplicate directions".into());
    }
    let grid_size = (views.len() as f64).sqrt().ceil() as u32;
    let edge = grid_size * tile_size;
    let mut rgba = vec![0_u8; edge as usize * edge as usize * 4];
    for (index, (_, view)) in ordered.iter().enumerate() {
        let tile_x = index as u32 % grid_size;
        let tile_y = index as u32 / grid_size;
        for row in 0..tile_size {
            let src = row as usize * tile_size as usize * 4;
            let dst = ((tile_y * tile_size + row) * edge + tile_x * tile_size) as usize * 4;
            rgba[dst..dst + tile_size as usize * 4]
                .copy_from_slice(&view.rgba[src..src + tile_size as usize * 4]);
        }
    }
    Ok(ImpostorAtlas {
        tile_size,
        grid_size,
        rgba,
        directions: ordered.iter().map(|(key, _)| *key).collect(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn triangle(asset: &str, x: f32) -> HlodInstance {
        HlodInstance {
            asset: AssetId::from_relative_path(asset),
            vertices: vec![
                Vertex {
                    position: [0.0, 0.0, 0.0],
                    normal: [0.0, 1.0, 0.0],
                    uv: [0.0, 0.0],
                },
                Vertex {
                    position: [1.0, 0.0, 0.0],
                    normal: [0.0, 1.0, 0.0],
                    uv: [1.0, 0.0],
                },
                Vertex {
                    position: [0.0, 0.0, 1.0],
                    normal: [0.0, 1.0, 0.0],
                    uv: [0.0, 1.0],
                },
            ],
            indices: vec![0, 1, 2],
            transform: Mat4::from_translation(Vec3::X * x),
            base_color: [x, 0.5, 0.25, 1.0],
        }
    }

    #[test]
    fn hlod_merges_transforms_materials_dependencies_and_budget() {
        let proxy = bake_hlod(&[triangle("b.glb", 1.0), triangle("a.glb", 3.0)], 1).unwrap();
        assert_eq!(proxy.indices.len(), 3);
        assert_eq!(proxy.vertices[0][0], 1.0);
        assert_eq!(proxy.base_color, [2.0, 0.5, 0.25, 1.0]);
        assert!(proxy.dependencies[0] < proxy.dependencies[1]);
    }

    #[test]
    fn octahedral_atlas_is_order_independent_and_complete() {
        let a = ImpostorView {
            direction: [1, 0, 1],
            rgba: vec![1; 16],
        };
        let b = ImpostorView {
            direction: [-1, 0, 1],
            rgba: vec![2; 16],
        };
        let first = bake_impostor(&[a.clone(), b.clone()], 2).unwrap();
        let second = bake_impostor(&[b, a], 2).unwrap();
        assert_eq!(first, second);
        assert_eq!(first.rgba.len(), 64);
    }
}
