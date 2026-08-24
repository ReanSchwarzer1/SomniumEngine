//! Phase CONTROL-O: deferred decals.
//!
//! # Marked optional, and shipped anyway
//!
//! §8 calls this a stretch and says the first thing to cut if anything slips.
//! Nothing slipped, so it is here — and it is here on the terms the plan set:
//! *"it ships with the drag gesture, or it does not ship."* Dragging a material
//! into the viewport with `Alt` held creates one of these, which is the
//! gesture, and it is expressible at all only because CONTROL-A1 put modifiers
//! on the input message.
//!
//! # What a decal is
//!
//! A box, and a material to project through it. The entity's `Transform`
//! supplies the box: its translation is the centre, its rotation is the
//! projection frame (the decal projects along its own **-Y**), and its scale is
//! the box's full size in metres — width, *depth of projection*, and height.
//!
//! That middle axis is the one people get wrong. It is not thickness for
//! looks; it is how far behind the surface the projection still applies, and
//! it is the only thing stopping a decal placed on a wall from also appearing
//! on the floor two metres below.
//!
//! # Where it is applied
//!
//! Inside the shading pass, over base colour, normal and roughness, before
//! anything is lit — because that is what "deferred decal" means and any other
//! placement would need the decal to re-light itself. It is binned into the
//! **same froxel grid as the local lights**: §8 names decals as the second
//! consumer 13C's clustering was shaped for, so `cluster.rs` grew a trait
//! rather than a copy.

use somnium_ecs::Component;

/// A projected material.
///
/// Sits on an entity beside a [`Transform`](crate::Transform) and a
/// [`MaterialComponent`](crate::MaterialComponent) — the transform is the box
/// and the material is what is projected, so a decal reuses the whole material
/// authoring surface CONTROL-D built rather than growing a parallel one.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DecalComponent {
    /// Draw this decal at all.
    pub enabled: bool,
    /// Overall opacity, `0..1`.
    pub opacity: f32,
    /// Draw order. Higher wins where two decals overlap.
    ///
    /// An explicit number rather than creation order, because creation order
    /// is not stable across a save and two decals that swapped every time the
    /// scene reloaded would be unauthorable.
    pub priority: i32,
    /// How far a surface's normal may tip from the decal's projection axis
    /// before the decal fades out, in degrees.
    ///
    /// The characteristic failure of naïve deferred decals is a projection
    /// aimed at the floor smearing down every wall inside its box. This is the
    /// fix, and it is authored rather than fixed because a blast scorch wants
    /// a wide angle and a poster wants almost none.
    pub angle_fade_degrees: f32,
    /// Strength of the material's normal map through the decal, `0..1`.
    pub normal_strength: f32,
    /// Roughness the decal writes where it is fully opaque.
    ///
    /// Separate from the material's own roughness because a decal is usually a
    /// *wet* or *scorched* patch on something, and the interesting authored
    /// value is what it does to the surface rather than what the source
    /// material happens to say.
    pub roughness: f32,
}

impl Component for DecalComponent {}

impl Default for DecalComponent {
    fn default() -> Self {
        Self {
            enabled: true,
            opacity: 1.0,
            priority: 0,
            angle_fade_degrees: 60.0,
            normal_strength: 1.0,
            roughness: 0.6,
        }
    }
}

/// The default box a freshly dropped decal gets, in metres.
///
/// Two metres across and half a metre of projection depth: big enough to see
/// where it landed and shallow enough that it does not immediately paint the
/// floor as well as the wall. Both are then draggable with the ordinary scale
/// gizmo, because a decal's box *is* its transform.
pub const DEFAULT_SIZE: [f32; 3] = [2.0, 0.5, 2.0];

/// The transform a decal dropped at `hit` with surface normal `normal` wants.
///
/// The projection axis is the decal's -Y, so the box is rotated to put its +Y
/// along the surface normal — a decal dropped on a floor projects downward and
/// one dropped on a wall projects into it, with no further authoring.
#[must_use]
pub fn placement(hit: glam::Vec3, normal: glam::Vec3) -> crate::Transform {
    let up = normal.normalize_or(glam::Vec3::Y);
    // Any rotation taking +Y onto the surface normal will do; the decal's
    // in-plane orientation is arbitrary until somebody rotates it, and picking
    // the shortest arc keeps a floor decal axis-aligned rather than spun by
    // whatever the reference vector happened to be.
    let rotation = glam::Quat::from_rotation_arc(glam::Vec3::Y, up);
    crate::Transform {
        // Lifted by a quarter of the projection depth so the box straddles the
        // surface rather than sitting entirely behind it — a decal flush with
        // the geometry z-fights its own volume boundary.
        translation: hit + up * (DEFAULT_SIZE[1] * 0.25),
        rotation,
        scale: glam::Vec3::from(DEFAULT_SIZE),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_floor_decal_projects_downward() {
        let t = placement(glam::Vec3::new(3.0, 0.0, -1.0), glam::Vec3::Y);
        // The decal projects along its own -Y, so +Y must be the surface
        // normal. Getting this backwards makes every decal invisible, because
        // the angle fade then rejects the surface it was dropped on.
        let axis = t.rotation.mul_vec3(glam::Vec3::Y);
        assert!((axis - glam::Vec3::Y).length() < 1e-5, "{axis}");
        assert!(t.translation.y > 0.0, "the box must straddle the surface");
    }

    #[test]
    fn a_wall_decal_projects_into_the_wall() {
        let normal = glam::Vec3::X;
        let t = placement(glam::Vec3::ZERO, normal);
        let axis = t.rotation.mul_vec3(glam::Vec3::Y);
        assert!((axis - normal).length() < 1e-5, "{axis}");
    }

    #[test]
    fn a_degenerate_normal_still_produces_a_usable_decal() {
        // A raycast that returns a zero normal must not produce a NaN
        // transform that then poisons the scene's bounding boxes.
        let t = placement(glam::Vec3::ZERO, glam::Vec3::ZERO);
        assert!(t.rotation.is_finite());
        assert!(t.translation.is_finite());
        assert!(t.scale.is_finite());
    }

    #[test]
    fn the_default_box_has_real_depth() {
        // Zero depth is a decal that can never be inside anything.
        assert!(DEFAULT_SIZE[1] > 0.0);
        assert!(DEFAULT_SIZE[0] > DEFAULT_SIZE[1], "wider than it is deep");
    }
}
