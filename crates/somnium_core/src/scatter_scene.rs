//! Convert authored scatter rules into ordinary undoable scene creations.

use crate::{Name, Transform, WorldTransform, editor_commands::EntitySnapshot};
use glam::{Quat, Vec2, Vec3};
use somnium_asset::scatter::{GradientImage, ScatterRule, SurfacePoint};
use somnium_ecs::{Entity, PersistentId, World};
use std::collections::BTreeMap;

/// Editor region for graph preview/bake. A source-local profile takes priority
/// over the first scene profile; a scene without one uses these defaults.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ScatterSettings {
    /// World width along X, in metres.
    pub width: f32,
    /// World depth along Z, in metres.
    pub depth: f32,
    /// Region centre offset in world X/Z metres from the selected source.
    pub offset: Vec2,
}
impl Default for ScatterSettings {
    fn default() -> Self {
        Self {
            width: 20.0,
            depth: 20.0,
            offset: Vec2::ZERO,
        }
    }
}
impl somnium_ecs::Component for ScatterSettings {}
impl ScatterSettings {
    /// Resolve the selected source's profile or the scene's shared profile.
    #[must_use]
    pub fn for_source(world: &World, source: Entity) -> Self {
        world
            .get::<Self>(source)
            .copied()
            .or_else(|| {
                world
                    .entities()
                    .find_map(|entity| world.get::<Self>(entity).copied())
            })
            .unwrap_or_default()
    }

    /// Validated half-open XZ bounds consumed by both Preview and Apply.
    pub fn region(self, origin: Vec3) -> Result<(Vec2, Vec2), String> {
        if !self.offset.is_finite()
            || !origin.is_finite()
            || !self.width.is_finite()
            || !self.depth.is_finite()
            || self.width <= 0.0
            || self.depth <= 0.0
        {
            return Err("Scatter Settings requires finite offsets and positive width/depth".into());
        }
        let center = Vec2::new(origin.x, origin.z) + self.offset;
        let half = Vec2::new(self.width, self.depth) * 0.5;
        Ok((center - half, center + half))
    }
}

/// Author-defined classifications supplied by a ground entity. Missing weights
/// default to one; repeated names keep their highest finite weight.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct SurfaceTagsComponent {
    /// Authored tag names; every surface additionally supplies `ground`.
    pub names: Vec<String>,
    /// Optional weights corresponding to the tag names.
    pub weights: Vec<f32>,
}
impl somnium_ecs::Component for SurfaceTagsComponent {}
impl SurfaceTagsComponent {
    /// Resolve classifications into finite weights, preserving the strongest
    /// weight when an author repeats a name.
    #[must_use]
    pub fn tags(&self) -> BTreeMap<String, f32> {
        let mut tags = BTreeMap::from([("ground".into(), 1.0_f32)]);
        for (index, name) in self.names.iter().enumerate() {
            let value = self.weights.get(index).copied().unwrap_or(1.0);
            if !name.trim().is_empty() && value.is_finite() {
                tags.entry(name.clone())
                    .and_modify(|old| *old = old.max(value.clamp(0.0, 1.0)))
                    .or_insert(value.clamp(0.0, 1.0));
            }
        }
        tags
    }
}

pub(crate) fn register(registry: &mut somnium_ecs::TypeRegistry) {
    registry.register(somnium_ecs::component_schema! {
        ScatterSettings as "somnium.ScatterSettings", display "Scatter Settings", version 1,
        fields {
            width {doc:"Preview/bake width in world metres, centred around the selected source."},
            depth {doc:"Preview/bake depth in world metres."},
            offset {doc:"World X/Z offset from the selected source to the scatter region centre."},
        }
    });
    registry.register(somnium_ecs::component_schema! {
        SurfaceTagsComponent as "somnium.SurfaceTags", display "Surface Tags", version 1,
        fields {
            names {doc:"Ground classifications consumed by scatter Surface Tag nodes."},
            weights {doc:"Weights per name in [0,1]. Missing weights default to one."},
        }
    });
}

/// Bake a selected mesh into ordinary entity snapshots. The caller groups
/// creations into one undo step. Source identity, parents and prefab links are
/// intentionally not duplicated; each result is an independent authored mesh.
pub fn bake(
    world: &World,
    source: Entity,
    rule: &ScatterRule,
    min: Vec2,
    max: Vec2,
    images: &BTreeMap<String, GradientImage>,
    surface: impl FnMut(Vec2) -> Option<SurfacePoint>,
) -> Result<Vec<EntitySnapshot>, String> {
    let mesh = world
        .get::<crate::MeshComponent>(source)
        .copied()
        .ok_or("Select a mesh entity to scatter")?;
    let source_transform = world.get::<WorldTransform>(source).map_or_else(
        || world.get::<Transform>(source).copied().unwrap_or_default(),
        |world_transform| {
            let (scale, rotation, translation) = world_transform.0.to_scale_rotation_translation();
            Transform {
                scale,
                rotation,
                translation,
            }
        },
    );
    let material = world.get::<crate::MaterialComponent>(source).copied();
    let kind = world.get::<crate::MeshKind>(source).copied();
    let blockout = world
        .get::<crate::blockout::BlockoutComponent>(source)
        .copied();
    if kind.is_none() && blockout.is_none() {
        return Err("Select a procedural mesh or Blockout to scatter. Imported meshes need a durable source asset before scattered copies can survive scene reload.".into());
    }
    let instances = rule.scatter(min, max, images, surface)?;
    Ok(instances
        .into_iter()
        .enumerate()
        .map(|(index, instance)| {
            let transform = Transform {
                translation: instance.position,
                rotation: Quat::from_rotation_arc(Vec3::Y, instance.normal)
                    * Quat::from_rotation_y(instance.yaw)
                    * source_transform.rotation,
                scale: source_transform.scale * instance.scale,
            };
            EntitySnapshot {
                name: Some(Name::new(&format!("Scatter {}", index + 1))),
                transform: Some(transform),
                wt: Some(WorldTransform(transform.to_matrix())),
                mesh: Some(mesh),
                mat: material,
                mesh_kind: kind,
                blockout,
                persistent_id: Some(PersistentId::mint()),
                ..Default::default()
            }
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use somnium_asset::scatter::Gradient;
    #[test]
    fn bake_preserves_mesh_intent_and_assigns_distinct_durable_identity() {
        let mut world = World::new();
        let source = world.spawn((
            Transform::default(),
            crate::MeshKind::Cube,
            crate::MeshComponent {
                vertex_offset: 0,
                index_offset: 0,
                index_count: 36,
            },
        ));
        let rule = ScatterRule {
            gradient: Gradient::Constant(1.0),
            seed: 1,
            spacing: 1.0,
            density: 1.0,
            jitter: 0.0,
            scale_min: 1.0,
            scale_max: 1.0,
            max_instances: 100,
        };
        let snapshots = bake(
            &world,
            source,
            &rule,
            Vec2::ZERO,
            Vec2::splat(2.0),
            &BTreeMap::new(),
            |p| {
                Some(SurfacePoint {
                    position: Vec3::new(p.x, 0.0, p.y),
                    normal: Vec3::Y,
                    tags: BTreeMap::new(),
                })
            },
        )
        .unwrap();
        assert_eq!(snapshots.len(), 4);
        let ids: std::collections::HashSet<_> =
            snapshots.iter().map(|s| s.persistent_id.unwrap()).collect();
        assert_eq!(ids.len(), 4);
        for snapshot in snapshots {
            let entity = snapshot.respawn(&mut world);
            assert!(world.get::<crate::MeshKind>(entity).is_some());
            assert!(world.persistent_id(entity).is_some());
        }
    }
}
