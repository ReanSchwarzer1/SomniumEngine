//! Geometry-staged planar reflections. One active mirror reuses posed vertices;
//! callers provide reverse-wound indices and an occluding alcove around its opening.
use crate::{Transform, WorldTransform};
use glam::{Mat4, Vec3};
use somnium_ecs::{Component, Entity, World, component_schema, reflect::TypeRegistry};

#[derive(Clone, Copy, Debug)]
/// A finite hero mirror with explicit cost and staging controls.
pub struct StagedMirror {
    /// Enable reflection drawing.
    pub enabled: bool,
    /// Opening dimensions in metres, centred on local origin.
    pub width: f32,
    /// Opening height in metres.
    pub height: f32,
    /// Distance behind the opening reserved for reflected geometry.
    pub depth: f32,
    /// Maximum eye distance at which this mirror draws the reflected body.
    pub active_distance: f32,
    /// Reveal the reflection only when this entity's WorkTarget completes.
    pub requires_repair: bool,
    /// Runtime triangle count submitted by the game's reflection adapter.
    pub rendered_triangles: u32,
}
impl Component for StagedMirror {}
impl Default for StagedMirror {
    fn default() -> Self {
        Self {
            enabled: true,
            width: 2.8,
            height: 2.8,
            depth: 12.0,
            active_distance: 8.0,
            requires_repair: false,
            rendered_triangles: 0,
        }
    }
}
/// Register authoring, persistence and runtime cost fields.
pub fn register(registry: &mut TypeRegistry) {
    registry.register(component_schema! {
        StagedMirror as "somnium.StagedMirror", display "Staged Mirror", version 1,
        fields {
            enabled {group:"Reflection"},
            width {group:"Opening",unit:"m",min:1.0,max:8.0},
            height {group:"Opening",unit:"m",min:1.0,max:5.0},
            depth {group:"Staging",unit:"m",min:4.0,max:30.0,doc:"Keep the mirrored alcove clear of unrelated geometry."},
            active_distance {group:"Staging",unit:"m",min:1.0,max:12.0,doc:"One nearest, front-facing mirror draws. Keep this below alcove depth."},
            requires_repair {group:"Reflection",doc:"Reveal after this entity's Burn / Repair target completes."},
            rendered_triangles {read_only:true,group:"Runtime",flags:FieldFlags::EDIT.union(FieldFlags::SCRIPT_READ)},
        }
    });
}
#[derive(Clone, Copy, Debug)]
/// A selected mirror and its world reflection transform.
pub struct MirrorView {
    /// Entity owning the opening.
    pub entity: Entity,
    /// Matrix with determinant -1; draw with reverse-wound indices.
    pub reflection: Mat4,
}
/// Reflect points and directions across a plane; reject a degenerate normal.
pub fn reflection_matrix(point: Vec3, normal: Vec3) -> Option<Mat4> {
    let n = normal.try_normalize()?;
    if !point.is_finite() {
        return None;
    }
    Some(Mat4::from_cols(
        (Vec3::X - 2.0 * n.x * n).extend(0.0),
        (Vec3::Y - 2.0 * n.y * n).extend(0.0),
        (Vec3::Z - 2.0 * n.z * n).extend(0.0),
        (2.0 * point.dot(n) * n).extend(1.0),
    ))
}
/// Reverse each triangle's winding for a reflected draw, retaining source indices.
pub fn reversed_indices(indices: &[u32]) -> Vec<u32> {
    indices
        .chunks_exact(3)
        .flat_map(|t| [t[0], t[2], t[1]])
        .collect()
}
/// Select the nearest front-side opening intersecting the current view frustum.
/// Pass the current unjittered view-projection matrix. A missing repair target
/// stays closed; staging geometry remains responsible for scene occlusion.
pub fn active(world: &World, eye: Vec3, view_projection: Mat4) -> Option<MirrorView> {
    world
        .entities()
        .filter_map(|entity| {
            let m = world.get::<StagedMirror>(entity)?;
            if !m.enabled
                || crate::is_hidden(world, entity)
                || (m.requires_repair
                    && !world
                        .get::<crate::work::WorkTarget>(entity)
                        .is_some_and(|w| w.complete))
            {
                return None;
            }
            let t = world.get::<Transform>(entity)?;
            let pose = world
                .get::<WorldTransform>(entity)
                .map_or_else(|| t.to_matrix(), |t| t.0);
            if !pose.is_finite()
                || pose.determinant() == 0.0
                || !opening_in_view(pose, *m, view_projection)
            {
                return None;
            }
            let at = pose.transform_point3(Vec3::ZERO);
            let n = pose
                .inverse()
                .transpose()
                .transform_vector3(Vec3::Z)
                .try_normalize()?;
            let distance = eye.distance(at);
            if (eye - at).dot(n) <= 0.05 || distance > m.active_distance.min(m.depth - 1.0) {
                return None;
            }
            Some((
                distance,
                MirrorView {
                    entity,
                    reflection: reflection_matrix(at, n)?,
                },
            ))
        })
        .min_by(|a, b| a.0.total_cmp(&b.0))
        .map(|v| v.1)
}

// Homogeneous clip-space plane rejection handles off-centre openings and
// openings spanning the near plane without dividing by a negative/zero w.
fn opening_in_view(pose: Mat4, m: StagedMirror, view_projection: Mat4) -> bool {
    if !view_projection.is_finite()
        || !m.width.is_finite()
        || !m.height.is_finite()
        || m.width <= 0.0
        || m.height <= 0.0
    {
        return false;
    }
    let clip = view_projection * pose;
    let corners = [(-1.0, -1.0), (-1.0, 1.0), (1.0, -1.0), (1.0, 1.0)]
        .map(|(x, y)| clip * Vec3::new(x * m.width * 0.5, y * m.height * 0.5, 0.0).extend(1.0));
    if corners.iter().any(|p| !p.is_finite()) {
        return false;
    }
    !(0..6).any(|plane| {
        corners.iter().all(|p| {
            (match plane {
                0 => p.x + p.w,
                1 => p.w - p.x,
                2 => p.y + p.w,
                3 => p.w - p.y,
                4 => p.z,
                _ => p.w - p.z,
            }) < 0.0
        })
    })
}
/// Box staging shared by example and game. Origin is opening centre; +Z faces
/// the real room. The surrounding front mask prevents reflected geometry leaking.
pub fn alcove_boxes(m: StagedMirror) -> Vec<(&'static str, Vec3, Vec3)> {
    let w = m.width.max(1.0);
    let h = m.height.max(1.0);
    let d = m.depth.max(4.0);
    vec![
        (
            "Mirror stage floor",
            Vec3::new(0.0, -h / 2.0 - 0.1, -d / 2.0),
            Vec3::new(w + 0.4, 0.2, d),
        ),
        (
            "Mirror stage ceiling",
            Vec3::new(0.0, h / 2.0 + 0.1, -d / 2.0),
            Vec3::new(w + 0.4, 0.2, d),
        ),
        (
            "Mirror stage left",
            Vec3::new(-w / 2.0 - 0.1, 0.0, -d / 2.0),
            Vec3::new(0.2, h, d),
        ),
        (
            "Mirror stage right",
            Vec3::new(w / 2.0 + 0.1, 0.0, -d / 2.0),
            Vec3::new(0.2, h, d),
        ),
        (
            "Mirror stage back",
            Vec3::new(0.0, 0.0, -d),
            Vec3::new(w, h, 0.2),
        ),
        (
            "Mirror left surround",
            Vec3::new(-w / 2.0 - 8.0, 0.0, 0.0),
            Vec3::new(16.0, 16.0, 0.15),
        ),
        (
            "Mirror right surround",
            Vec3::new(w / 2.0 + 8.0, 0.0, 0.0),
            Vec3::new(16.0, 16.0, 0.15),
        ),
        (
            "Mirror top surround",
            Vec3::new(0.0, h / 2.0 + 8.0, 0.0),
            Vec3::new(w, 16.0, 0.15),
        ),
        (
            "Mirror bottom surround",
            Vec3::new(0.0, -h / 2.0 - 8.0, 0.0),
            Vec3::new(w, 16.0, 0.15),
        ),
    ]
}
#[cfg(test)]
mod tests {
    use super::*;
    fn projection(eye: Vec3, target: Vec3) -> Mat4 {
        Mat4::perspective_rh(60.0_f32.to_radians(), 1.0, 0.1, 100.0)
            * Mat4::look_at_rh(eye, target, Vec3::Y)
    }
    #[test]
    fn selection_ignores_nearer_openings_outside_the_view() {
        let mut world = World::new();
        let expected = world.spawn((
            Transform::from_translation(Vec3::new(0.0, 0.0, -5.0)),
            StagedMirror::default(),
        ));
        // Front side faces the eye, but the entire opening is behind the camera.
        world.spawn((
            Transform {
                translation: Vec3::new(0.0, 0.0, 1.0),
                rotation: glam::Quat::from_rotation_y(std::f32::consts::PI),
                ..Default::default()
            },
            StagedMirror::default(),
        ));
        // In front of the camera but entirely offscreen and closer than expected.
        world.spawn((
            Transform::from_translation(Vec3::new(3.0, 0.0, -1.0)),
            StagedMirror::default(),
        ));
        let view = projection(Vec3::ZERO, Vec3::NEG_Z);
        assert_eq!(active(&world, Vec3::ZERO, view).unwrap().entity, expected);
        world
            .insert_component(
                expected,
                crate::EditorFlags {
                    hidden: true,
                    ..Default::default()
                },
            )
            .unwrap();
        assert!(active(&world, Vec3::ZERO, view).is_none());
    }
    #[test]
    fn sheared_mirror_reflects_about_its_transformed_surface() {
        let mut world = World::new();
        let pose = Mat4::from_scale(Vec3::new(2.0, 1.0, 1.0)) * Mat4::from_rotation_y(0.6);
        let normal = pose
            .inverse()
            .transpose()
            .transform_vector3(Vec3::Z)
            .normalize();
        let eye = normal * 4.0;
        world.spawn((
            Transform::default(),
            WorldTransform(pose),
            StagedMirror::default(),
        ));
        let mirror = active(&world, eye, projection(eye, Vec3::ZERO)).unwrap();
        for local in [Vec3::ZERO, Vec3::X, Vec3::Y, Vec3::new(-0.6, 0.8, 0.0)] {
            let on_plane = pose.transform_point3(local);
            assert!(
                mirror
                    .reflection
                    .transform_point3(on_plane)
                    .distance(on_plane)
                    < 1e-5
            );
        }
        assert!(mirror.reflection.transform_point3(eye).distance(-eye) < 1e-5);
    }
    #[test]
    fn opening_clipping_keeps_large_intersections_and_rejects_closed_repairs() {
        let mut world = World::new();
        let entity = world.spawn((
            Transform::from_translation(Vec3::new(0.0, 0.0, -1.0)),
            StagedMirror {
                width: 8.0,
                height: 5.0,
                requires_repair: true,
                ..Default::default()
            },
            crate::work::WorkTarget::default(),
        ));
        let view = projection(Vec3::ZERO, Vec3::NEG_Z);
        assert!(active(&world, Vec3::ZERO, view).is_none());
        world
            .get_mut::<crate::work::WorkTarget>(entity)
            .unwrap()
            .complete = true;
        assert_eq!(active(&world, Vec3::ZERO, view).unwrap().entity, entity);
    }
    #[test]
    fn reflected_pose_is_involutory_and_changes_handedness() {
        let at = Vec3::new(2.0, 0.0, -3.0);
        let n = Vec3::new(0.6, 0.0, 0.8);
        let r = reflection_matrix(at, n).unwrap();
        for p in [at, at + n * 2.0, Vec3::new(-1.0, 1.7, 4.0)] {
            assert!(r.transform_point3(r.transform_point3(p)).distance(p) < 1e-5);
            assert!(((r.transform_point3(p) - at).dot(n) + (p - at).dot(n)).abs() < 1e-5);
        }
        assert!((r.determinant() + 1.0).abs() < 1e-5);
        assert_eq!(reversed_indices(&[0, 1, 2, 2, 3, 0]), [0, 2, 1, 2, 0, 3]);
        assert!(reflection_matrix(at, Vec3::ZERO).is_none());
    }
}
